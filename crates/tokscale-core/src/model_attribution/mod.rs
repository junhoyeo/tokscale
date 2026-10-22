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
pub mod diagnostics;
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

/// Trim the retained ledger in batches instead of after every appended row.
///
/// Once the cap is reached, dropping one element per append from the front of
/// a million-entry `Vec` is a quadratic scan of the whole ledger. Instead the
/// buffer is allowed to grow by this many records past the cap and the oldest
/// batch is removed in one shift, which makes the amortized cost per line
/// constant and keeps memory bounded by `cap + this`.
const LEDGER_TRIM_BATCH: usize = 4096;

#[cfg(test)]
thread_local! {
    /// Test-only override so the trim path can be exercised with a small log.
    static LEDGER_CAP_OVERRIDE: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
    /// Number of front drains performed by [`read_ledger`] on this thread.
    /// Batching makes this proportional to rows/batch, not to rows.
    static LEDGER_DRAINS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn ledger_record_cap() -> usize {
    #[cfg(test)]
    {
        if let Some(cap) = LEDGER_CAP_OVERRIDE.with(|value| value.get()) {
            return cap;
        }
    }
    MAX_LEDGER_RECORDS
}

/// Retained-record cap with the batching slack applied.
fn ledger_record_limit() -> usize {
    ledger_record_cap().saturating_add(LEDGER_TRIM_BATCH.min(ledger_record_cap()))
}

/// Bound on messages parked for cross-lane one-to-one claim resolution. A
/// scan beyond it attributes nothing at all: attributing the rows that happen
/// to fit would make the result depend on lane order. Reaching the cap is a
/// resource limit, not evidence, so no row is silently preferred.
const MAX_PARKED_MESSAGES: usize = 250_000;

#[cfg(test)]
thread_local! {
    /// Test-only override so the cap's fail-closed behavior can be exercised
    /// without parking a quarter million rows.
    static PARK_CAP_OVERRIDE: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

fn park_cap() -> usize {
    #[cfg(test)]
    {
        if let Some(cap) = PARK_CAP_OVERRIDE.with(|value| value.get()) {
            return cap;
        }
    }
    MAX_PARKED_MESSAGES
}

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
            let limit = ledger_record_limit();
            if records.len() > limit {
                // Amortized: this branch removes at least one batch, so the
                // following batch of appends pays no shift at all.
                let excess = records.len() - ledger_record_cap();
                records.drain(..excess);
                #[cfg(test)]
                LEDGER_DRAINS.with(|count| count.set(count.get() + 1));
            }
        }
    }
    let cap = ledger_record_cap();
    if records.len() > cap {
        let excess = records.len() - cap;
        records.drain(..excess);
        #[cfg(test)]
        LEDGER_DRAINS.with(|count| count.set(count.get() + 1));
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
    let index = Arc::new(GatewayIndex::new(policy, records));
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    struct CapReset;

    impl Drop for CapReset {
        fn drop(&mut self) {
            LEDGER_CAP_OVERRIDE.with(|value| value.set(None));
        }
    }

    /// The trim path must keep the newest `cap` records and stay linear. With
    /// a tiny cap this exercises the batched drain repeatedly instead of
    /// requiring a million-row fixture.
    #[test]
    fn an_oversized_ledger_keeps_only_its_newest_records() {
        let _reset = CapReset;
        LEDGER_CAP_OVERRIDE.with(|value| value.set(Some(100)));
        assert_eq!(ledger_record_cap(), 100);
        // With a cap smaller than the batch size, the batch is clamped so the
        // buffer never grows past twice the cap.
        assert_eq!(ledger_record_limit(), 200);

        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("ledger.jsonl");
        let mut file = std::fs::File::create(&path).unwrap();
        let total = 10_000i64;
        for index in 0..total {
            writeln!(file, "{index}").unwrap();
        }
        drop(file);

        let records = read_ledger(&path, |line| {
            std::str::from_utf8(line).ok()?.parse::<i64>().ok()
        })
        .unwrap();
        assert_eq!(records.len(), 100);
        assert_eq!(records.first().copied(), Some(total - 100));
        assert_eq!(records.last().copied(), Some(total - 1));
    }

    /// Batching is what keeps the amortized cost per appended row constant:
    /// the number of drains must be proportional to `records / batch`, never
    /// to the number of appended records.
    #[test]
    fn ledger_trimming_drains_in_batches_not_once_per_row() {
        let _reset = CapReset;
        LEDGER_CAP_OVERRIDE.with(|value| value.set(Some(64)));
        LEDGER_DRAINS.with(|count| count.set(0));
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("ledger.jsonl");
        let mut file = std::fs::File::create(&path).unwrap();
        let total = 20_000i64;
        for index in 0..total {
            writeln!(file, "{index}").unwrap();
        }
        drop(file);

        let records = read_ledger(&path, |line| {
            std::str::from_utf8(line).ok()?.parse::<i64>().ok()
        })
        .unwrap();
        let drains = LEDGER_DRAINS.with(std::cell::Cell::get);
        assert_eq!(records.len(), 64);
        assert_eq!(records.last().copied(), Some(total - 1));
        // A one-per-row drain at 20k rows would need ~20k shifts. Batching
        // removes a whole batch at a time, so the observed count is the number
        // of batches, not the number of rows.
        let batches = (total / 64) as usize;
        assert!(
            drains <= batches && drains >= batches / 2,
            "expected one drain per batch near {batches}, observed {drains}"
        );
        assert!(
            drains < total as usize / 8,
            "batch trimming must not degrade to per-row drains, observed {drains}"
        );
    }

    /// A malformed or oversized row never hides valid rows after it.
    #[test]
    fn ledger_reading_skips_noise_without_dropping_valid_rows() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("ledger.jsonl");
        std::fs::write(&path, "{bad}\n\n  \n7\n").unwrap();
        let records = read_ledger(&path, |line| {
            std::str::from_utf8(line).ok()?.parse::<i64>().ok()
        })
        .unwrap();
        assert_eq!(records, vec![7]);
    }
}
