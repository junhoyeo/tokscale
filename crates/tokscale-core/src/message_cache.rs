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
use std::time::UNIX_EPOCH;

const CACHE_SCHEMA_VERSION: u32 = 7;
const CACHE_FILENAME: &str = "source-message-cache.bin";
const CACHE_LOCK_FILENAME: &str = "source-message-cache.lock";
const MAX_CACHE_FILE_BYTES: u64 = 256 * 1024 * 1024;
const FINGERPRINT_SAMPLE_BYTES: usize = 4096;
const FINGERPRINT_SAMPLE_POINTS: usize = 5;
const HASH_BUFFER_BYTES: usize = 64 * 1024;

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

fn cache_path() -> Option<PathBuf> {
    Some(cache_dir()?.join(CACHE_FILENAME))
}

fn cache_lock_path() -> Option<PathBuf> {
    Some(cache_dir()?.join(CACHE_LOCK_FILENAME))
}

fn legacy_cache_paths() -> Vec<PathBuf> {
    if crate::paths::is_config_dir_overridden() {
        return Vec::new();
    }

    [
        crate::paths::legacy_dirs_cache_dir().map(|d| d.join(CACHE_FILENAME)),
        crate::paths::legacy_dot_cache_tokscale_dir().map(|d| d.join(CACHE_FILENAME)),
    ]
    .into_iter()
    .flatten()
    .collect()
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RelatedFileFingerprint {
    pub suffix: String,
    pub size: u64,
    pub modified_ns: u64,
    pub sample_hashes: Vec<FileSampleHash>,
    pub content_hash: [u8; 32],
}

impl SourceFingerprint {
    pub(crate) fn from_path(path: &Path) -> Option<Self> {
        Self::from_path_with_related(path, std::iter::empty())
    }

    pub(crate) fn from_sqlite_path(path: &Path) -> Option<Self> {
        let related_paths = ["-wal"]
            .into_iter()
            .map(|suffix| (suffix.to_string(), append_path_suffix(path, suffix)));
        Self::from_path_with_related(path, related_paths)
    }

    /// Fingerprint for a Claude Code JSONL file that may have a sibling `.meta.json`
    /// sidecar. When the sidecar appears or changes (e.g. after a Claude Code upgrade),
    /// the fingerprint changes and the cache invalidates.
    pub(crate) fn from_claude_code_path(path: &Path) -> Option<Self> {
        let related = path.file_stem().and_then(|s| s.to_str()).map(|stem| {
            let meta_filename = format!("{}.meta.json", stem);
            let meta_path = path.with_file_name(meta_filename);
            (".meta.json".to_string(), meta_path)
        });
        Self::from_path_with_related(path, related)
    }

    fn from_path_with_related<I>(path: &Path, related_paths: I) -> Option<Self>
    where
        I: IntoIterator<Item = (String, PathBuf)>,
    {
        let (size, modified_ns, sample_hashes, content_hash) = file_fingerprint_parts(path)?;
        let mut related_files: Vec<RelatedFileFingerprint> = related_paths
            .into_iter()
            .filter_map(|(suffix, related_path)| {
                RelatedFileFingerprint::from_path(suffix, &related_path)
            })
            .collect();
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
    fn from_path(suffix: String, path: &Path) -> Option<Self> {
        let (size, modified_ns, sample_hashes, content_hash) = file_fingerprint_parts(path)?;
        Some(Self {
            suffix,
            size,
            modified_ns,
            sample_hashes,
            content_hash,
        })
    }
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct CachedSourceEntry {
    pub path: CachedPath,
    pub fingerprint: SourceFingerprint,
    pub messages: Vec<UnifiedMessage>,
    pub fallback_timestamp_indices: Vec<usize>,
    pub codex_incremental: Option<CodexIncrementalCache>,
}

impl CachedSourceEntry {
    pub(crate) fn new(
        path: &Path,
        fingerprint: SourceFingerprint,
        messages: Vec<UnifiedMessage>,
        fallback_timestamp_indices: Vec<usize>,
        codex_incremental: Option<CodexIncrementalCache>,
    ) -> Self {
        Self {
            path: CachedPath::from_path(path),
            fingerprint,
            messages,
            fallback_timestamp_indices,
            codex_incremental,
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct CachedSourceStore {
    schema_version: u32,
    entries: Vec<CachedSourceEntry>,
}

#[derive(Default)]
pub(crate) struct SourceMessageCache {
    pub entries: HashMap<CachedPath, CachedSourceEntry>,
    dirty: bool,
    dirty_keys: HashSet<CachedPath>,
    deleted_paths: HashSet<CachedPath>,
}

impl SourceMessageCache {
    pub(crate) fn load() -> Self {
        let Some(path) = cache_path() else {
            return Self::default();
        };
        let Some(lock_path) = cache_lock_path() else {
            return Self::default();
        };
        if let Some(lock_dir) = lock_path.parent() {
            if ensure_cache_dir(lock_dir).is_err() {
                return Self::default();
            }
        }
        let lock_file = match OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path)
        {
            Ok(file) => file,
            Err(_) => return Self::default(),
        };
        if fs2::FileExt::lock_shared(&lock_file).is_err() {
            return Self::default();
        }

        let store = std::iter::once(path)
            .chain(legacy_cache_paths())
            .find_map(|path| read_store_from_path(&path));
        let Some(store) = store else {
            return Self::default();
        };

        let entries = store
            .entries
            .into_iter()
            .map(|entry| (entry.path.clone(), entry))
            .collect();

        Self {
            entries,
            dirty: false,
            dirty_keys: HashSet::new(),
            deleted_paths: HashSet::new(),
        }
    }

    pub(crate) fn insert(&mut self, entry: CachedSourceEntry) {
        let key = entry.path.clone();
        self.entries.insert(key.clone(), entry);
        self.deleted_paths.remove(&key);
        self.dirty_keys.insert(key);
        self.dirty = true;
    }

    pub(crate) fn get(&self, path: &Path) -> Option<&CachedSourceEntry> {
        let key = CachedPath::from_path(path);
        self.entries.get(&key)
    }

    pub(crate) fn prune_missing_files(&mut self) {
        let removed_paths: Vec<CachedPath> = self
            .entries
            .keys()
            .filter(|path| !path.to_path_buf().exists())
            .cloned()
            .collect();
        if removed_paths.is_empty() {
            return;
        }

        for path in removed_paths {
            self.entries.remove(&path);
            self.dirty_keys.remove(&path);
            self.deleted_paths.insert(path);
        }
        self.dirty = true;
    }

    pub(crate) fn save_if_dirty(&mut self) {
        if !self.dirty {
            return;
        }

        let Some(dir) = cache_dir() else {
            return;
        };
        if ensure_cache_dir(&dir).is_err() {
            return;
        }

        let Some(final_path) = cache_path() else {
            return;
        };
        let Some(lock_path) = cache_lock_path() else {
            return;
        };
        let lock_file = match OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path)
        {
            Ok(file) => file,
            Err(_) => return,
        };
        if fs2::FileExt::lock_exclusive(&lock_file).is_err() {
            return;
        }

        let mut merged_entries: HashMap<CachedPath, CachedSourceEntry> =
            read_store_from_path(&final_path)
                .map(|store| {
                    store
                        .entries
                        .into_iter()
                        .map(|entry| (entry.path.clone(), entry))
                        .collect()
                })
                .unwrap_or_default();

        for path in &self.deleted_paths {
            merged_entries.remove(path);
        }
        for path in &self.dirty_keys {
            if let Some(entry) = self.entries.get(path) {
                merged_entries.insert(path.clone(), entry.clone());
            }
        }

        let store = CachedSourceStore {
            schema_version: CACHE_SCHEMA_VERSION,
            entries: merged_entries.values().cloned().collect(),
        };

        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        let tmp_path = dir.join(format!(
            ".{}.{}.{:x}.tmp",
            CACHE_FILENAME,
            std::process::id(),
            nanos
        ));

        // INVARIANT: All cache writes use atomic temp-file rename. NEVER delete
        // the canonical cache file before writing — a partial save or process
        // crash between delete and rename would lose the cache. The temp-file
        // pattern makes corruption-on-crash impossible.
        let write_result = (|| -> std::io::Result<()> {
            let file = File::create(&tmp_path)?;
            let mut writer = BufWriter::new(file);
            bincode::options()
                .with_limit(MAX_CACHE_FILE_BYTES)
                .serialize_into(&mut writer, &store)
                .map_err(std::io::Error::other)?;
            writer.flush()?;
            writer.get_ref().sync_all()?;
            if fs::rename(&tmp_path, &final_path).is_err() {
                fs::copy(&tmp_path, &final_path)?;
                let final_file = File::open(&final_path)?;
                final_file.sync_all()?;
                let _ = fs::remove_file(&tmp_path);
            }
            Ok(())
        })();

        if write_result.is_err() {
            let _ = fs::remove_file(&tmp_path);
            return;
        }

        self.entries = merged_entries;
        self.dirty = false;
        self.dirty_keys.clear();
        self.deleted_paths.clear();
    }
}

fn read_store_from_path(path: &Path) -> Option<CachedSourceStore> {
    let file = File::open(path).ok()?;
    let metadata = file.metadata().ok()?;
    if metadata.len() > MAX_CACHE_FILE_BYTES {
        return None;
    }

    let reader = BufReader::new(file);
    let store: CachedSourceStore = bincode::options()
        .with_limit(MAX_CACHE_FILE_BYTES)
        .deserialize_from(reader)
        .ok()?;
    if store.schema_version != CACHE_SCHEMA_VERSION {
        return None;
    }
    Some(store)
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

fn file_fingerprint_parts(path: &Path) -> Option<(u64, u64, Vec<FileSampleHash>, [u8; 32])> {
    let metadata = path.metadata().ok()?;
    let size = metadata.len();
    let modified_ns = metadata
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_nanos() as u64;
    let sample_hashes = compute_sample_hashes(path, size)?;
    let content_hash = hash_prefix(path, size)?;
    Some((size, modified_ns, sample_hashes, content_hash))
}

fn append_path_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut os = OsString::from(path.as_os_str());
    os.push(suffix);
    PathBuf::from(os)
}

fn hash_prefix(path: &Path, len: u64) -> Option<[u8; 32]> {
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

pub(crate) fn build_codex_incremental_cache(
    path: &Path,
    consumed_offset: u64,
    state: CodexParseState,
) -> Option<CodexIncrementalCache> {
    Some(CodexIncrementalCache {
        state,
        consumed_offset,
        ends_with_newline: consumed_offset == 0 || file_ends_with_newline(path, consumed_offset),
        prefix_hash: hash_prefix(path, consumed_offset)?,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TokenBreakdown;
    use std::io::Write;
    use tempfile::{NamedTempFile, TempDir};

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
    /// `XDG_CACHE_HOME` from the host, in which case `paths::get_cache_dir`
    /// resolves outside the sandbox and the legacy fallback never gets
    /// exercised. Returns the previous values so the caller can restore.
    fn sandbox_cache_env(
        temp_home: &std::path::Path,
    ) -> (
        Option<std::ffi::OsString>,
        Option<std::ffi::OsString>,
        Option<std::ffi::OsString>,
    ) {
        let prev_home = std::env::var_os("HOME");
        let prev_xdg_config = std::env::var_os("XDG_CONFIG_HOME");
        let prev_xdg_cache = std::env::var_os("XDG_CACHE_HOME");
        unsafe {
            std::env::set_var("HOME", temp_home);
            std::env::set_var("XDG_CONFIG_HOME", temp_home.join(".config"));
            std::env::set_var("XDG_CACHE_HOME", temp_home.join(".cache"));
        }
        (prev_home, prev_xdg_config, prev_xdg_cache)
    }

    fn restore_cache_env(
        prev: (
            Option<std::ffi::OsString>,
            Option<std::ffi::OsString>,
            Option<std::ffi::OsString>,
        ),
    ) {
        restore_env_var("HOME", prev.0);
        restore_env_var("XDG_CONFIG_HOME", prev.1);
        restore_env_var("XDG_CACHE_HOME", prev.2);
    }

    fn write_temp_file(content: &[u8]) -> NamedTempFile {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(content).unwrap();
        file.flush().unwrap();
        file
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
    fn test_claude_code_fingerprint_tracks_meta_sidecar_changes() {
        let dir = TempDir::new().unwrap();
        let jsonl_path = dir.path().join("agent-abc123.jsonl");
        std::fs::write(&jsonl_path, b"jsonl-content").unwrap();

        // No meta sidecar → baseline fingerprint
        let base = SourceFingerprint::from_claude_code_path(&jsonl_path).unwrap();

        // Add meta sidecar → fingerprint changes
        let meta_path = dir.path().join("agent-abc123.meta.json");
        std::fs::write(&meta_path, br#"{"agentType":"explore"}"#).unwrap();
        let with_meta = SourceFingerprint::from_claude_code_path(&jsonl_path).unwrap();
        assert_ne!(
            base, with_meta,
            "Adding meta sidecar should change fingerprint"
        );

        // Update meta sidecar → fingerprint changes again
        std::fs::write(&meta_path, br#"{"agentType":"executor"}"#).unwrap();
        let updated_meta = SourceFingerprint::from_claude_code_path(&jsonl_path).unwrap();
        assert_ne!(
            with_meta, updated_meta,
            "Updating meta sidecar should change fingerprint"
        );

        // Main session file (no agent- prefix) → unaffected by unrelated meta files
        let main_path = dir.path().join("session-uuid.jsonl");
        std::fs::write(&main_path, b"main-session").unwrap();
        let main_fp1 = SourceFingerprint::from_claude_code_path(&main_path).unwrap();
        // Create a meta file with the main session stem (unlikely in practice)
        let main_meta = dir.path().join("session-uuid.meta.json");
        std::fs::write(&main_meta, br#"{"agentType":"x"}"#).unwrap();
        let main_fp2 = SourceFingerprint::from_claude_code_path(&main_path).unwrap();
        assert_ne!(
            main_fp1, main_fp2,
            "from_claude_code_path always tracks .meta.json if it exists"
        );
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
    fn test_source_message_cache_round_trip() {
        let temp_home = TempDir::new().unwrap();
        let prev_env = sandbox_cache_env(temp_home.path());

        let file = write_temp_file(b"{}\n");
        let fingerprint = SourceFingerprint::from_path(file.path()).unwrap();
        let entry = CachedSourceEntry::new(
            file.path(),
            fingerprint,
            vec![UnifiedMessage::new(
                "client",
                "gpt-5",
                "provider",
                "session-1",
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
        );

        let mut cache = SourceMessageCache::default();
        cache.insert(entry);
        cache.save_if_dirty();

        let loaded = SourceMessageCache::load();
        assert_eq!(loaded.entries.len(), 1);
        assert!(loaded.get(file.path()).is_some());

        restore_cache_env(prev_env);
    }

    #[test]
    fn test_prune_missing_files_removes_deleted_entries() {
        let file = write_temp_file(b"{}\n");
        let fingerprint = SourceFingerprint::from_path(file.path()).unwrap();
        let path = file.path().to_path_buf();

        let mut cache = SourceMessageCache::default();
        cache.insert(CachedSourceEntry::new(
            &path,
            fingerprint,
            Vec::new(),
            Vec::new(),
            None,
        ));

        std::fs::remove_file(&path).unwrap();
        cache.prune_missing_files();

        assert!(cache.entries.is_empty());
    }

    #[test]
    #[serial_test::serial]
    fn test_load_ignores_oversized_cache_file() {
        let temp_home = TempDir::new().unwrap();
        let original_home = std::env::var("HOME").ok();
        restore_env_var("HOME", Some(temp_home.path()));

        {
            let cache_file = cache_path().unwrap();
            ensure_cache_dir(cache_file.parent().unwrap()).unwrap();
            let file = File::create(&cache_file).unwrap();
            file.set_len(MAX_CACHE_FILE_BYTES + 1).unwrap();

            let loaded = SourceMessageCache::load();
            assert!(loaded.entries.is_empty());
        }

        restore_env_var("HOME", original_home);
    }

    #[test]
    #[serial_test::serial]
    fn test_load_ignores_stale_schema_version() {
        let temp_home = TempDir::new().unwrap();
        let original_home = std::env::var("HOME").ok();
        restore_env_var("HOME", Some(temp_home.path()));

        {
            let cache_file = cache_path().unwrap();
            ensure_cache_dir(cache_file.parent().unwrap()).unwrap();
            let store = CachedSourceStore {
                schema_version: CACHE_SCHEMA_VERSION - 1,
                entries: Vec::new(),
            };

            let writer = BufWriter::new(File::create(&cache_file).unwrap());
            bincode::options().serialize_into(writer, &store).unwrap();

            let loaded = SourceMessageCache::load();
            assert!(loaded.entries.is_empty());
        }

        restore_env_var("HOME", original_home);
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
        let original_home = std::env::var("HOME").ok();
        restore_env_var("HOME", Some(temp_home.path()));

        let mut cache = SourceMessageCache::default();
        assert!(!cache.dirty);

        {
            let file = write_temp_file(b"{}\n");
            let fingerprint = SourceFingerprint::from_path(file.path()).unwrap();
            cache.insert(CachedSourceEntry::new(
                file.path(),
                fingerprint,
                Vec::new(),
                Vec::new(),
                None,
            ));
            assert!(cache.dirty);

            cache.save_if_dirty();
            assert!(!cache.dirty);
        }

        restore_env_var("HOME", original_home);
    }

    #[test]
    #[serial_test::serial]
    fn test_save_if_dirty_merges_concurrent_writers() {
        let temp_home = TempDir::new().unwrap();
        let original_home = std::env::var("HOME").ok();
        restore_env_var("HOME", Some(temp_home.path()));

        {
            let file_one = write_temp_file(b"{\"id\":1}\n");
            let file_two = write_temp_file(b"{\"id\":2}\n");

            let mut writer_one = SourceMessageCache::load();
            let mut writer_two = SourceMessageCache::load();

            writer_one.insert(CachedSourceEntry::new(
                file_one.path(),
                SourceFingerprint::from_path(file_one.path()).unwrap(),
                Vec::new(),
                Vec::new(),
                None,
            ));
            writer_two.insert(CachedSourceEntry::new(
                file_two.path(),
                SourceFingerprint::from_path(file_two.path()).unwrap(),
                Vec::new(),
                Vec::new(),
                None,
            ));

            writer_one.save_if_dirty();
            writer_two.save_if_dirty();

            let loaded = SourceMessageCache::load();
            assert!(loaded.get(file_one.path()).is_some());
            assert!(loaded.get(file_two.path()).is_some());
        }

        restore_env_var("HOME", original_home);
    }

    #[test]
    #[serial_test::serial]
    fn load_falls_back_to_legacy_dirs_cache_path() {
        let temp_home = TempDir::new().unwrap();
        let temp_xdg_cache = TempDir::new().unwrap();
        let original_home = std::env::var_os("HOME");
        let original_xdg_cache = std::env::var_os("XDG_CACHE_HOME");
        let original_xdg_config = std::env::var_os("XDG_CONFIG_HOME");
        let original_override = std::env::var_os("TOKSCALE_CONFIG_DIR");

        restore_env_var("HOME", Some(temp_home.path()));
        restore_env_var("XDG_CACHE_HOME", Some(temp_xdg_cache.path()));
        restore_env_var("XDG_CONFIG_HOME", Some(temp_home.path().join(".config")));
        restore_env_var("TOKSCALE_CONFIG_DIR", None::<&str>);

        let source = write_temp_file(b"legacy-dirs\n");
        let entry = CachedSourceEntry::new(
            source.path(),
            SourceFingerprint::from_path(source.path()).unwrap(),
            Vec::new(),
            Vec::new(),
            None,
        );

        let legacy_path = crate::paths::legacy_dirs_cache_dir()
            .unwrap()
            .join(CACHE_FILENAME);
        ensure_cache_dir(legacy_path.parent().unwrap()).unwrap();
        let store = CachedSourceStore {
            schema_version: CACHE_SCHEMA_VERSION,
            entries: vec![entry],
        };
        let writer = BufWriter::new(File::create(&legacy_path).unwrap());
        bincode::options().serialize_into(writer, &store).unwrap();

        let loaded = SourceMessageCache::load();
        assert!(loaded.get(source.path()).is_some());

        restore_env_var("HOME", original_home);
        restore_env_var("XDG_CACHE_HOME", original_xdg_cache);
        restore_env_var("XDG_CONFIG_HOME", original_xdg_config);
        restore_env_var("TOKSCALE_CONFIG_DIR", original_override);
    }

    #[test]
    #[serial_test::serial]
    fn load_falls_back_to_legacy_dot_cache_path() {
        let temp_home = TempDir::new().unwrap();
        let original_home = std::env::var_os("HOME");
        let original_xdg_cache = std::env::var_os("XDG_CACHE_HOME");
        let original_xdg_config = std::env::var_os("XDG_CONFIG_HOME");
        let original_override = std::env::var_os("TOKSCALE_CONFIG_DIR");

        restore_env_var("HOME", Some(temp_home.path()));
        restore_env_var("XDG_CACHE_HOME", None::<&str>);
        restore_env_var("XDG_CONFIG_HOME", Some(temp_home.path().join(".config")));
        restore_env_var("TOKSCALE_CONFIG_DIR", None::<&str>);

        let source = write_temp_file(b"legacy-dot\n");
        let entry = CachedSourceEntry::new(
            source.path(),
            SourceFingerprint::from_path(source.path()).unwrap(),
            Vec::new(),
            Vec::new(),
            None,
        );

        let legacy_path = crate::paths::legacy_dot_cache_tokscale_dir()
            .unwrap()
            .join(CACHE_FILENAME);
        ensure_cache_dir(legacy_path.parent().unwrap()).unwrap();
        let store = CachedSourceStore {
            schema_version: CACHE_SCHEMA_VERSION,
            entries: vec![entry],
        };
        let writer = BufWriter::new(File::create(&legacy_path).unwrap());
        bincode::options().serialize_into(writer, &store).unwrap();

        let loaded = SourceMessageCache::load();
        assert!(loaded.get(source.path()).is_some());

        restore_env_var("HOME", original_home);
        restore_env_var("XDG_CACHE_HOME", original_xdg_cache);
        restore_env_var("XDG_CONFIG_HOME", original_xdg_config);
        restore_env_var("TOKSCALE_CONFIG_DIR", original_override);
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
