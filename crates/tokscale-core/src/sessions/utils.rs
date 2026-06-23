//! Shared parsing helpers for session logs.

use rusqlite::{Connection, OpenFlags};
use serde_json::Value;
use std::path::Path;
use std::time::SystemTime;

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
        Some(numeric * 1000)
    }
}

pub(crate) fn parse_timestamp_str(value: &str) -> Option<i64> {
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(value) {
        return Some(dt.timestamp_millis());
    }

    if let Ok(numeric) = value.parse::<i64>() {
        if numeric <= 0 {
            return None;
        }
        if numeric >= 1_000_000_000_000 {
            return Some(numeric);
        }
        return Some(numeric * 1000);
    }

    None
}

pub(crate) fn file_modified_timestamp_ms(path: &Path) -> i64 {
    std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|time| time.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_else(|| chrono::Utc::now().timestamp_millis())
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
