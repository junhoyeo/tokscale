//! Evidence-based model attribution shared by gateway adapters.
//!
//! A client records what it asked for; a gateway records what it actually
//! contacted and what the upstream response said served the request. Those are
//! different identities and must never be conflated. This module keeps the
//! evidence in one normalized shape ([`matcher::GatewayRecord`]), correlates
//! one gateway record with at most one parsed usage row
//! ([`matcher::GatewayIndex`]), resolves the surviving evidence into a single
//! physical identity or a conflict ([`resolver`]), and applies that identity to
//! an existing [`UnifiedMessage`](crate::sessions::UnifiedMessage) without
//! changing any counted token or creating a second request row ([`apply`]).
//!
//! Adapters own their persistence formats; nothing here parses a log. Adding a
//! gateway means decoding its records into `GatewayRecord`s and registering an
//! index with the parse flush, not copying the matching policy.
//!
//! Evidence kinds, strongest first:
//!
//! 1. the served model reported by the completed upstream response;
//! 2. a confirmed gateway selection, when the adapter declares it admissible
//!    and no response-reported model contradicts it;
//! 3. an explicit request/response identifier, which establishes that a record
//!    and a message are the same call but not which model served it;
//! 4. a native session id plus a verified token signature, or the requested
//!    selector plus a verified token signature and timing.
//!
//! A requested router label is never evidence of the physical model. When the
//! evidence does not converge, the original identity stands and the row is
//! marked conflicted rather than priced as a guess.

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::SystemTime;

pub(crate) mod apply;
pub(crate) mod diagnostics;
pub(crate) mod magpie;
pub(crate) mod matcher;
pub(crate) mod overlay;
pub(crate) mod resolver;

pub(crate) use matcher::{GatewayIndex, GatewayRecord, MatchPolicy};
pub(crate) use overlay::GatewayOverlay;

/// One hand-edited or truncated ledger line must not allocate without bound.
pub(crate) const MAX_LEDGER_LINE_BYTES: usize = 1024 * 1024;

/// A gateway ledger larger than this is truncated to its newest records, so a
/// corrupt log cannot consume unbounded memory. Attribution then fails closed
/// for records that were dropped.
const MAX_LEDGER_RECORDS: usize = 1_000_000;

/// Bound on messages parked for cross-lane one-to-one claim resolution. A
/// scan beyond it releases the remaining messages unattributed, which is the
/// same fail-closed outcome as a missing record.
const MAX_PARKED_MESSAGES: usize = 250_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BoundedLine {
    Eof,
    Line,
    Oversized,
}

/// Read and consume one JSONL row without allocating beyond the row cap. An
/// oversized row is discarded through its newline and parsing continues.
pub(crate) fn read_bounded_line<R: BufRead>(
    reader: &mut R,
    line: &mut Vec<u8>,
) -> io::Result<BoundedLine> {
    line.clear();
    let mut consumed = 0usize;
    let mut oversized = false;

    loop {
        let (take, has_newline) = {
            let available = reader.fill_buf()?;
            if available.is_empty() {
                return Ok(if consumed == 0 {
                    BoundedLine::Eof
                } else if oversized {
                    BoundedLine::Oversized
                } else {
                    BoundedLine::Line
                });
            }
            let take = available
                .iter()
                .position(|byte| *byte == b'\n')
                .map_or(available.len(), |index| index + 1);
            if !oversized {
                if line.len().saturating_add(take) > MAX_LEDGER_LINE_BYTES {
                    oversized = true;
                    line.clear();
                } else {
                    line.extend_from_slice(&available[..take]);
                }
            }
            (take, available[take - 1] == b'\n')
        };
        reader.consume(take);
        consumed = consumed.saturating_add(take);
        if has_newline {
            if !oversized {
                if line.last() == Some(&b'\n') {
                    line.pop();
                    if line.last() == Some(&b'\r') {
                        line.pop();
                    }
                }
                return Ok(BoundedLine::Line);
            }
            return Ok(BoundedLine::Oversized);
        }
    }
}

/// Read every bounded JSONL row in `path`, letting `decode` turn one line into
/// zero or one record. Malformed and oversized lines are skipped; they never
/// hide a valid row.
pub(crate) fn read_ledger<T>(
    path: &Path,
    mut decode: impl FnMut(&[u8]) -> Option<T>,
) -> io::Result<Vec<T>> {
    let file = File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut line = Vec::new();
    let mut records = Vec::new();

    loop {
        match read_bounded_line(&mut reader, &mut line)? {
            BoundedLine::Eof => break,
            BoundedLine::Oversized => continue,
            BoundedLine::Line => {}
        }
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        if let Some(record) = decode(&line) {
            records.push(record);
            if records.len() > MAX_LEDGER_RECORDS {
                records.drain(..records.len() - MAX_LEDGER_RECORDS);
            }
        }
    }
    Ok(records)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LedgerFingerprint {
    len: u64,
    modified: Option<SystemTime>,
}

struct CachedIndex {
    fingerprint: LedgerFingerprint,
    index: Arc<GatewayIndex>,
}

/// One cache per ledger path. The fingerprint is the file's length and
/// modification time, so an appended row invalidates the cached index instead
/// of leaving stale attribution behind.
static LEDGER_INDEX_CACHE: LazyLock<Mutex<HashMap<PathBuf, CachedIndex>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub(crate) fn cached_index(
    path: &Path,
    adapter: &'static str,
    policy: MatchPolicy,
    decode: impl FnOnce(&Path) -> Option<Vec<GatewayRecord>>,
) -> Option<Arc<GatewayIndex>> {
    let metadata = std::fs::metadata(path).ok()?;
    let fingerprint = LedgerFingerprint {
        len: metadata.len(),
        modified: metadata.modified().ok(),
    };

    {
        let cache = LEDGER_INDEX_CACHE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(cached) = cache.get(path) {
            if cached.fingerprint == fingerprint {
                return Some(Arc::clone(&cached.index));
            }
        }
    }

    let records = decode(path)?;
    let index = Arc::new(GatewayIndex::new(adapter, policy, records));
    let mut cache = LEDGER_INDEX_CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    cache.insert(
        path.to_path_buf(),
        CachedIndex {
            fingerprint,
            index: Arc::clone(&index),
        },
    );
    Some(index)
}

#[cfg(test)]
pub(crate) fn clear_index_cache() {
    LEDGER_INDEX_CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clear();
}
