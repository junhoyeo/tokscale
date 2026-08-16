//! Shared parsing helpers for session logs.

use crate::TokenBreakdown;
use rusqlite::{Connection, OpenFlags};
use serde::Deserialize;
use serde_json::Value;
use std::io::BufRead;
use std::path::Path;
use std::time::SystemTime;
use tracing::warn;

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

/// Iterate lossy-decoded lines while retaining their exact source bytes.
///
/// Most parsers need only [`lossy_lines`]. Prime Agent additionally derives a
/// stable fallback deduplication key from corrupted records, where hashing the
/// decoded string would collapse distinct invalid UTF-8 sequences to the same
/// replacement character.
pub(crate) fn lossy_lines_with_bytes<R: BufRead>(reader: R) -> LossyLinesWithBytes<R> {
    LossyLinesWithBytes {
        reader,
        buf: Vec::new(),
        at_start: true,
    }
}

pub(crate) struct LossyLine {
    pub text: String,
    pub bytes: Vec<u8>,
}

pub(crate) struct LossyLines<R> {
    reader: R,
    buf: Vec<u8>,
    at_start: bool,
}

/// Read one line into `buf`, stripping the line terminator and a leading BOM.
///
/// Returns the offset in `buf` at which the line's payload starts, so callers
/// can borrow it without allocating, or `None` at end of input.
fn read_line_payload<R: BufRead>(
    reader: &mut R,
    buf: &mut Vec<u8>,
    at_start: &mut bool,
) -> Option<usize> {
    buf.clear();
    match reader.read_until(b'\n', buf) {
        Ok(0) => None,
        Ok(_) => {
            if buf.last() == Some(&b'\n') {
                buf.pop();
                if buf.last() == Some(&b'\r') {
                    buf.pop();
                }
            }

            let bom = "\u{feff}".as_bytes();
            if std::mem::take(at_start) && buf.starts_with(bom) {
                Some(bom.len())
            } else {
                Some(0)
            }
        }
        // A hard I/O error (vanished mount, EIO) does not consume input, so
        // retrying would spin on the same failing read forever. Stop instead.
        Err(_) => None,
    }
}

fn read_lossy_line<R: BufRead>(
    reader: &mut R,
    buf: &mut Vec<u8>,
    at_start: &mut bool,
) -> Option<String> {
    let start = read_line_payload(reader, buf, at_start)?;
    Some(String::from_utf8_lossy(&buf[start..]).into_owned())
}

/// Read `path` as line-delimited JSON, handing every non-blank, trimmed line to
/// `sink` together with its zero-based physical line index.
///
/// A missing or unreadable file yields nothing, which is what every JSONL
/// parser already did by hand.
///
/// This also fixes a silent-truncation bug wherever it replaces
/// `BufReader::lines()`. That iterator ends on the first line that is not valid
/// UTF-8, so a single stray byte discarded the entire rest of a transcript —
/// #1031 measured ~2% of an 83 MB Grok `updates.jsonl` surviving. Decoding
/// lossily per line keeps the damage to the line carrying the bad byte, and
/// the index stays aligned with the physical file so callers that report line
/// positions still agree with it.
///
/// The line is borrowed out of a reused buffer rather than allocated per line,
/// so callers that need an owned value must clone it themselves.
///
/// `sink` is `&mut dyn FnMut` and not a generic `impl FnMut` on purpose: a type
/// parameter would monomorphize this driver once per calling parser and grow
/// the binary, which is what sharing it exists to avoid.
pub(crate) fn for_each_json_line(path: &Path, sink: &mut dyn FnMut(usize, &str)) {
    let Ok(file) = std::fs::File::open(path) else {
        return;
    };

    let mut reader = std::io::BufReader::new(file);
    let mut buf = Vec::new();
    let mut at_start = true;
    let mut index = 0usize;

    while let Some(start) = read_line_payload(&mut reader, &mut buf, &mut at_start) {
        let text = String::from_utf8_lossy(&buf[start..]);
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            sink(index, trimmed);
        }
        index += 1;
    }
}

impl<R: BufRead> Iterator for LossyLines<R> {
    type Item = String;

    fn next(&mut self) -> Option<Self::Item> {
        read_lossy_line(&mut self.reader, &mut self.buf, &mut self.at_start)
    }
}

pub(crate) struct LossyLinesWithBytes<R> {
    reader: R,
    buf: Vec<u8>,
    at_start: bool,
}

impl<R: BufRead> Iterator for LossyLinesWithBytes<R> {
    type Item = LossyLine;

    fn next(&mut self) -> Option<Self::Item> {
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

                Some(LossyLine {
                    text: String::from_utf8_lossy(bytes).into_owned(),
                    bytes: bytes.to_vec(),
                })
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
///
/// The `NO_MUTEX` flag is safe here because each parser uses its connection on
/// one thread. Returning the original `rusqlite::Error` lets callers preserve
/// useful open-failure context in their logs.
pub(crate) fn open_readonly_sqlite(path: &Path) -> rusqlite::Result<Connection> {
    Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
}

/// Open a SQLite file for read-only access, discarding open errors.
/// Returns `None` if the file cannot be opened — the caller treats that as "no sessions".
pub(crate) fn open_readonly_sqlite_opt(path: &Path) -> Option<Connection> {
    open_readonly_sqlite(path).ok()
}

/// Which stage of a [`sqlite_for_each_row`] scan the driver reached.
///
/// A bare `bool` is not enough: parsers that keep an older query around as a
/// fallback need "this database does not have that schema" (`prepare` failed,
/// so try the next query) to be distinguishable from "the query ran", while
/// parsers that degrade to a coarser data source need "rows were actually
/// iterated".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SqliteScan {
    /// The statement prepared and its rows were iterated — possibly zero rows.
    Ran,
    /// The database could not be opened.
    NotOpened,
    /// `prepare` rejected the statement, normally a missing table or column.
    NotPrepared,
    /// The statement prepared but the query could not execute.
    NotExecuted,
}

impl SqliteScan {
    /// True only when rows were iterated.
    pub(crate) fn ran(self) -> bool {
        matches!(self, SqliteScan::Ran)
    }

    /// True unless `prepare` rejected the statement. Callers that fall back to
    /// an older schema use this to stop at the first query the database
    /// understands, even when executing it then failed — an execute failure
    /// means the schema matched and something else went wrong, so retrying an
    /// older query would silently read the wrong columns.
    pub(crate) fn prepared(self) -> bool {
        !matches!(self, SqliteScan::NotPrepared | SqliteScan::NotOpened)
    }
}

/// Run `sql` on an already-open connection and hand every row to `sink`.
///
/// `what` labels the data being read (`"Goose session"`) in the warnings for
/// prepare, execute and row-decode failures. `None` scans silently, which is
/// what parsers probing an optional table want: a missing table there is the
/// expected case, not a fault worth logging on every run.
///
/// A row that `sink` rejects is skipped and the scan continues, matching
/// `query_map`'s behaviour where a per-row decode error does not end
/// iteration. An error stepping the statement does end it, because SQLite
/// closes the cursor at that point.
///
/// `sink` is `&mut dyn FnMut` and not a generic `impl FnMut` on purpose: a type
/// parameter would monomorphize this driver once per calling parser and grow
/// the binary, which is what sharing it exists to avoid.
pub(crate) fn sqlite_for_each_row_on(
    conn: &Connection,
    db_path: &Path,
    sql: &str,
    what: Option<&str>,
    sink: &mut dyn FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<()>,
) -> SqliteScan {
    let mut stmt = match conn.prepare(sql) {
        Ok(stmt) => stmt,
        Err(err) => {
            if let Some(what) = what {
                warn!(
                    db_path = %db_path.display(),
                    what,
                    error = %err,
                    "Failed to prepare session query"
                );
            }
            return SqliteScan::NotPrepared;
        }
    };

    let mut rows = match stmt.query([]) {
        Ok(rows) => rows,
        Err(err) => {
            if let Some(what) = what {
                warn!(
                    db_path = %db_path.display(),
                    what,
                    error = %err,
                    "Failed to execute session query"
                );
            }
            return SqliteScan::NotExecuted;
        }
    };

    loop {
        match rows.next() {
            Ok(Some(row)) => {
                if let Err(err) = sink(row) {
                    if let Some(what) = what {
                        warn!(
                            db_path = %db_path.display(),
                            what,
                            error = %err,
                            "Failed to decode session row"
                        );
                    }
                }
            }
            Ok(None) => break,
            Err(err) => {
                if let Some(what) = what {
                    warn!(
                        db_path = %db_path.display(),
                        what,
                        error = %err,
                        "Failed to decode session row"
                    );
                }
                break;
            }
        }
    }

    SqliteScan::Ran
}

/// Open `db_path` read-only, run `sql`, and hand every row to `sink`.
///
/// The connection-owning half of [`sqlite_for_each_row_on`], for the common
/// case of one query per database. Parsers that run several queries against
/// one database should open once and call [`sqlite_for_each_row_on`].
pub(crate) fn sqlite_for_each_row(
    db_path: &Path,
    sql: &str,
    what: Option<&str>,
    sink: &mut dyn FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<()>,
) -> SqliteScan {
    let conn = match open_readonly_sqlite(db_path) {
        Ok(conn) => conn,
        Err(err) => {
            if let Some(what) = what {
                warn!(
                    db_path = %db_path.display(),
                    what,
                    error = %err,
                    "Failed to open session database"
                );
            }
            return SqliteScan::NotOpened;
        }
    };
    sqlite_for_each_row_on(&conn, db_path, sql, what, sink)
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

/// Fallback token estimate for records that carry no usage metadata: one token
/// per four characters, rounded up.
pub(crate) fn estimate_tokens(chars: usize) -> i64 {
    chars.div_ceil(4) as i64
}

/// Session id taken from a transcript file's stem, e.g.
/// `.../ses_abc123.jsonl` -> `ses_abc123`.
///
/// Clients whose session id is not the file stem — or that treat a blank stem
/// differently — keep their own resolver rather than calling this.
pub(crate) fn session_id_from_path(path: &Path) -> String {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("unknown")
        .to_string()
}

/// Workspace key taken from the directory that contains a transcript file.
pub(crate) fn workspace_key_from_path(path: &Path) -> Option<String> {
    path.parent()
        .and_then(|dir| dir.file_name())
        .and_then(|name| name.to_str())
        .and_then(super::normalize_workspace_key)
}

/// Normalize an epoch timestamp to milliseconds.
///
/// A recent epoch is ~1.7e12 in milliseconds versus ~1.7e9 in seconds, so a
/// value at or under the `1e12` threshold is read as seconds and scaled up.
/// Scaling happens in `f64` to keep sub-second precision; the cast then clamps
/// into `i64` range so a garbage or huge timestamp saturates rather than
/// wrapping, and a `NaN` becomes `0`.
pub(crate) fn timestamp_secs_to_ms(timestamp: f64) -> i64 {
    if timestamp > 1e12 {
        timestamp as i64
    } else {
        let millis = timestamp * 1000.0;
        if millis.is_nan() {
            0
        } else {
            millis.clamp(i64::MIN as f64, i64::MAX as f64) as i64
        }
    }
}

/// Resolve a provider from the record's own provider name, falling back to
/// inference from the model id and finally to `fallback` (normally the client
/// id itself).
pub(crate) fn resolved_provider(
    provider: Option<String>,
    model_id: &str,
    fallback: &str,
) -> String {
    provider
        .filter(|provider| !provider.trim().is_empty())
        .and_then(|provider| crate::provider_identity::canonical_provider(provider.trim()))
        .or_else(|| {
            crate::provider_identity::inferred_provider_from_model(model_id).map(str::to_string)
        })
        .unwrap_or_else(|| fallback.to_string())
}

/// The Anthropic Messages API `usage` block in its snake_case wire spelling.
///
/// Shared by the clients that persist Anthropic responses verbatim
/// (`claudecode`, `augment`), which declared byte-identical copies of it.
///
/// Clients whose payload adds fields on top of this shape — a
/// `reasoning_output_tokens`, a `cached_input_tokens`, a `total_tokens` — keep
/// their own struct on purpose. Widening this one with aliases would make the
/// clients above start counting fields they deliberately ignore today, and
/// change reported totals.
/// Public because `claudecode` re-exports it as its historical `ClaudeUsage`;
/// `sessions::utils` itself is crate-private.
#[derive(Debug, Deserialize)]
pub struct AnthropicUsage {
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cache_read_input_tokens: Option<i64>,
    pub cache_creation_input_tokens: Option<i64>,
}

impl AnthropicUsage {
    /// Token breakdown with every field clamped at zero. This block carries no
    /// reasoning bucket, so `reasoning` is always 0.
    pub fn to_breakdown(&self) -> TokenBreakdown {
        TokenBreakdown {
            input: self.input_tokens.unwrap_or(0).max(0),
            output: self.output_tokens.unwrap_or(0).max(0),
            cache_read: self.cache_read_input_tokens.unwrap_or(0).max(0),
            cache_write: self.cache_creation_input_tokens.unwrap_or(0).max(0),
            reasoning: 0,
        }
    }
}

/// The camelCase `{input, output, cacheRead, cacheWrite, totalTokens}` usage
/// block with an authoritative `cost.total` in USD, as written by `gjc` and
/// `openclaw`. They declared the same struct twice, one spelling the rename
/// with `rename_all` and the other with per-field `rename`, so the JSON both
/// accept is identical.
///
/// `pi.rs` models the same wire shape with its own `PiUsage` (a flattened
/// extras map it needs and this type does not) and is left alone.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CamelUsage {
    pub(crate) input: Option<i64>,
    pub(crate) output: Option<i64>,
    pub(crate) cache_read: Option<i64>,
    pub(crate) cache_write: Option<i64>,
    /// Reported but unused: the breakdown is summed from the fields above, so
    /// a disagreeing total must not silently override them.
    #[allow(dead_code)]
    pub(crate) total_tokens: Option<i64>,
    pub(crate) cost: Option<CamelCost>,
}

impl CamelUsage {
    /// Token breakdown with every field clamped at zero. This block carries no
    /// reasoning bucket, so `reasoning` is always 0.
    pub(crate) fn to_breakdown(&self) -> TokenBreakdown {
        TokenBreakdown {
            input: self.input.unwrap_or(0).max(0),
            output: self.output.unwrap_or(0).max(0),
            cache_read: self.cache_read.unwrap_or(0).max(0),
            cache_write: self.cache_write.unwrap_or(0).max(0),
            reasoning: 0,
        }
    }
}

/// The `cost` sibling of [`CamelUsage`].
#[derive(Debug, Deserialize)]
pub(crate) struct CamelCost {
    /// Authoritative total cost in USD.
    pub(crate) total: Option<f64>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::ErrorCode;

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
    fn lossy_lines_with_bytes_preserves_distinct_invalid_sequences() {
        let raw: &[u8] = b"a\xff\na\xfe\n";
        let lines: Vec<LossyLine> = lossy_lines_with_bytes(raw).collect();
        assert_eq!(lines[0].text, lines[1].text);
        assert_eq!(lines[0].bytes, b"a\xff");
        assert_eq!(lines[1].bytes, b"a\xfe");
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

    #[test]
    fn open_readonly_sqlite_rejects_writes_but_reads_existing_data() {
        let temp_dir = tempfile::tempdir().unwrap();
        let db_path = temp_dir.path().join("state.db");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute("CREATE TABLE sessions (id TEXT)", []).unwrap();
        drop(conn);

        let conn = open_readonly_sqlite(&db_path).unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);

        let error = conn
            .execute("INSERT INTO sessions (id) VALUES ('session')", [])
            .unwrap_err();
        assert!(
            matches!(
                &error,
                rusqlite::Error::SqliteFailure(sqlite_error, _)
                    if sqlite_error.code == ErrorCode::ReadOnly
            ),
            "expected SQLITE_READONLY, got {error:?}"
        );
    }

    #[test]
    fn open_readonly_sqlite_preserves_cannot_open_error() {
        let temp_dir = tempfile::tempdir().unwrap();
        let db_path = temp_dir.path().join("missing.db");
        let error = open_readonly_sqlite(&db_path).unwrap_err();

        assert!(
            matches!(
                &error,
                rusqlite::Error::SqliteFailure(sqlite_error, _)
                    if sqlite_error.code == ErrorCode::CannotOpen
            ),
            "expected SQLITE_CANTOPEN, got {error:?}"
        );
    }
}
