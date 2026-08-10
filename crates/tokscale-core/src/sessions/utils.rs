//! Shared parsing helpers for session logs.

use rusqlite::{Connection, OpenFlags};
use serde_json::Value;
use std::io::BufRead;
use std::path::Path;
use std::time::SystemTime;

/// Iterate a reader line by line without letting one undecodable byte discard
/// the rest of the stream.
///
/// `BufRead::lines()` yields `Err(InvalidData)` for any line that is not valid
/// UTF-8, and the `map_while(Result::ok)` spelling turns that into
/// end-of-iteration: a single stray byte anywhere in a multi-megabyte session
/// log silently dropped every record after it (#1031 measured ~2% of an 83MB
/// Grok `updates.jsonl` surviving). Reading raw bytes up to each newline and
/// decoding them lossily keeps the cost of a bad byte local to its own line.
///
/// Line endings match `lines()`: the trailing `\n` and any preceding `\r` are
/// stripped, and a final line without a newline is still yielded.
pub(crate) fn lossy_lines<R: BufRead>(reader: R) -> LossyLines<R> {
    LossyLines {
        reader,
        buf: Vec::new(),
        at_start: true,
    }
}

pub(crate) struct LossyLines<R> {
    reader: R,
    buf: Vec<u8>,
    at_start: bool,
}

impl<R: BufRead> Iterator for LossyLines<R> {
    type Item = String;

    fn next(&mut self) -> Option<String> {
        self.buf.clear();
        match self.reader.read_until(b'\n', &mut self.buf) {
            Ok(0) => None,
            Ok(_) => {
                if self.buf.last() == Some(&b'\n') {
                    self.buf.pop();
                    if self.buf.last() == Some(&b'\r') {
                        self.buf.pop();
                    }
                }

                let mut bytes = self.buf.as_slice();
                if std::mem::take(&mut self.at_start) {
                    // A UTF-8 BOM decodes cleanly but leaves U+FEFF glued to the
                    // front of the first record, where it makes an otherwise
                    // valid JSON line fail to parse and be skipped in silence.
                    bytes = bytes.strip_prefix("\u{feff}".as_bytes()).unwrap_or(bytes);
                }

                Some(String::from_utf8_lossy(bytes).into_owned())
            }
            // Decode failures cannot reach this arm — lossy decoding never
            // fails — so an error here is a hard I/O failure (vanished network
            // mount, EIO). `read_until` does not consume input when it fails
            // that way, so skipping and retrying would spin on the same failing
            // read forever. Stop instead, and keep the lines read so far.
            Err(_) => None,
        }
    }
}

pub(crate) fn extract_i64(value: Option<&Value>) -> Option<i64> {
    value.and_then(|val| {
        val.as_i64()
            .or_else(|| val.as_u64().map(|v| v as i64))
            .or_else(|| val.as_str().and_then(|s| s.parse::<i64>().ok()))
    })
}

pub(crate) fn extract_string(value: Option<&Value>) -> Option<String> {
    value.and_then(|val| val.as_str().map(|s| s.to_string()))
}

pub(crate) fn parse_timestamp_value(value: &Value) -> Option<i64> {
    if let Some(ts) = value.as_str() {
        return parse_timestamp_str(ts);
    }

    let numeric = value
        .as_i64()
        .or_else(|| value.as_u64().map(|v| v as i64))?;
    if numeric <= 0 {
        return None;
    }
    if numeric >= 1_000_000_000_000 {
        Some(numeric)
    } else {
        // Seconds -> milliseconds: saturating so a garbage/huge timestamp
        // cannot overflow i64 during the conversion.
        Some(numeric.saturating_mul(1000))
    }
}

pub(crate) fn parse_timestamp_str(value: &str) -> Option<i64> {
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(value) {
        return Some(dt.timestamp_millis());
    }

    // Timezone-less ISO-8601 datetimes (e.g. "2026-06-16T12:00:00",
    // "2026-06-16 12:00:00", optional fractional seconds) carry no offset, so
    // `parse_from_rfc3339` rejects them. Interpret them as UTC rather than
    // collapsing to the file mtime, which would scatter the message into the
    // wrong day/month bucket.
    for format in [
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%d %H:%M:%S",
    ] {
        if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(value, format) {
            return Some(naive.and_utc().timestamp_millis());
        }
    }

    if let Ok(numeric) = value.parse::<i64>() {
        if numeric <= 0 {
            return None;
        }
        if numeric >= 1_000_000_000_000 {
            return Some(numeric);
        }
        // Seconds -> milliseconds: saturating so a garbage/huge timestamp
        // cannot overflow i64 during the conversion.
        return Some(numeric.saturating_mul(1000));
    }

    None
}

/// Modification time in epoch milliseconds, or `None` when the filesystem does
/// not report one.
///
/// `SystemTime::modified` is the one timestamp every tier-1 target supports
/// (unlike `created`, which is absent on many Linux filesystems), so callers
/// that need a portable "when was this last written" anchor use this.
///
/// Returns `None` rather than substituting a value so a caller with its own
/// fallback can reach for it instead of silently anchoring on the wrong
/// instant. Pre-epoch mtimes also collapse to `None`: `duration_since` fails
/// for them, and a negative anchor would bucket the record before 1970.
pub(crate) fn file_modified_timestamp_ms_opt(path: &Path) -> Option<i64> {
    std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|time| time.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis() as i64)
}

pub(crate) fn file_modified_timestamp_ms(path: &Path) -> i64 {
    file_modified_timestamp_ms_opt(path).unwrap_or_else(|| chrono::Utc::now().timestamp_millis())
}

/// Open a SQLite file for read-only access with no mutex (single-threaded parser use).
/// Returns `None` if the file cannot be opened — the caller treats that as "no sessions".
pub(crate) fn open_readonly_sqlite(path: &Path) -> Option<Connection> {
    Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()
}

/// Read a file into bytes, returning `None` on any I/O error instead of propagating.
/// Used by parsers that treat missing/unreadable session files as "no data".
pub(crate) fn read_file_or_none(path: &Path) -> Option<Vec<u8>> {
    std::fs::read(path).ok()
}

/// Back-calculate a start anchor from a recorded end timestamp and an elapsed
/// duration: `end - duration`.
///
/// Several session sources only record the timestamp at which a call/turn
/// *finished*, plus its elapsed duration. Anchoring the message at that end
/// timestamp directly would make `sessionize()`'s
/// `[timestamp, timestamp + duration_ms]` span project forward past the
/// actual completion into phantom idle time (see #890), so callers
/// back-calculate the start instead. That subtraction can itself produce a
/// non-positive result when `duration` exceeds `end` (e.g. a corrupt or
/// clock-skewed duration value) — `sessionize()` silently drops any message
/// with `timestamp <= 0`, so this guards against that by falling back to the
/// unadjusted `end` timestamp when the back-calculated candidate would not
/// be positive.
pub(crate) fn back_anchor_timestamp(end: i64, duration: i64) -> i64 {
    end.checked_sub(duration)
        .filter(|candidate| *candidate > 0)
        .unwrap_or(end)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lossy_lines_survives_undecodable_bytes_and_strips_a_bom() {
        let raw: &[u8] = b"\xef\xbb\xbffirst\r\nse\xffcond\nthird";
        let lines: Vec<String> = lossy_lines(raw).collect();
        assert_eq!(lines, vec!["first", "se\u{fffd}cond", "third"]);
    }

    #[test]
    fn lossy_lines_keeps_empty_lines_and_ends_at_eof() {
        let raw: &[u8] = b"a\n\nb\n";
        let lines: Vec<String> = lossy_lines(raw).collect();
        assert_eq!(lines, vec!["a", "", "b"]);
    }

    #[test]
    fn parse_timestamp_value_rejects_zero_and_negative_numbers() {
        assert!(parse_timestamp_value(&serde_json::json!(0)).is_none());
        assert!(parse_timestamp_value(&serde_json::json!(-1000)).is_none());
        assert!(parse_timestamp_value(&serde_json::json!(-1_700_000_000_000_i64)).is_none());
    }

    #[test]
    fn parse_timestamp_value_accepts_positive_numbers() {
        assert_eq!(
            parse_timestamp_value(&serde_json::json!(1_700_000_000_000_i64)),
            Some(1_700_000_000_000)
        );
        assert_eq!(
            parse_timestamp_value(&serde_json::json!(1_700_000_000_i64)),
            Some(1_700_000_000_000)
        );
    }

    #[test]
    fn parse_timestamp_str_rejects_zero_and_negative_strings() {
        assert!(parse_timestamp_str("0").is_none());
        assert!(parse_timestamp_str("-5").is_none());
    }

    #[test]
    fn parse_timestamp_str_accepts_timezone_less_datetimes_as_utc() {
        // "2026-06-16T12:00:00" UTC == 1781611200000 ms.
        assert_eq!(
            parse_timestamp_str("2026-06-16T12:00:00"),
            Some(1_781_611_200_000)
        );
        // Space separator and fractional seconds variants.
        assert_eq!(
            parse_timestamp_str("2026-06-16 12:00:00"),
            Some(1_781_611_200_000)
        );
        assert_eq!(
            parse_timestamp_str("2026-06-16T12:00:00.500"),
            Some(1_781_611_200_500)
        );
        // Offset-bearing input still goes through the rfc3339 path unchanged.
        assert_eq!(
            parse_timestamp_str("2026-06-16T12:00:00Z"),
            Some(1_781_611_200_000)
        );
    }
}
