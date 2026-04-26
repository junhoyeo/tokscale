//! Goose session parser
//!
//! Parses session rows from Goose's SQLite sessions database:
//! - Primary: `~/.local/share/goose/sessions/sessions.db`
//! - macOS: `~/Library/Application Support/goose/sessions/sessions.db`
//! - Legacy Block/goose: `~/.local/share/Block/goose/sessions/sessions.db`
//! - Custom: `$GOOSE_PATH_ROOT/data/sessions/sessions.db`

use super::UnifiedMessage;
use crate::{provider_identity, TokenBreakdown};
use rusqlite::Connection;
use serde::Deserialize;
use std::path::Path;
use tracing::warn;

#[derive(Debug, Deserialize)]
struct GooseModelConfig {
    model_name: String,
}

fn parse_model_config(json: &str) -> Option<String> {
    let mut bytes = json.as_bytes().to_vec();
    let config: GooseModelConfig = simd_json::from_slice(&mut bytes).ok()?;
    let name = config.model_name.trim().to_string();
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

fn timestamp_secs_to_ms(timestamp: f64) -> i64 {
    if timestamp > 1e12 {
        timestamp as i64
    } else {
        (timestamp * 1000.0) as i64
    }
}

fn resolved_provider(provider_name: Option<String>, model_id: &str) -> String {
    provider_name
        .filter(|p| !p.trim().is_empty())
        .and_then(|p| provider_identity::canonical_provider(p.trim()))
        .or_else(|| provider_identity::inferred_provider_from_model(model_id).map(str::to_string))
        .unwrap_or_else(|| "goose".to_string())
}

fn parse_created_at(s: &str) -> f64 {
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return dt.timestamp_millis() as f64;
    }
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S") {
        return dt.and_utc().timestamp_millis() as f64;
    }
    if let Ok(date) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return date
            .and_hms_opt(0, 0, 0)
            .unwrap_or_default()
            .and_utc()
            .timestamp_millis() as f64;
    }
    0.0
}

pub fn parse_goose_sqlite(db_path: &Path) -> Vec<UnifiedMessage> {
    let conn = match Connection::open_with_flags(
        db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) {
        Ok(c) => c,
        Err(err) => {
            warn!(
                db_path = %db_path.display(),
                error = %err,
                "Failed to open Goose sessions database"
            );
            return Vec::new();
        }
    };

    let query = r#"
        SELECT
            id,
            model_config_json,
            provider_name,
            created_at,
            total_tokens,
            input_tokens,
            output_tokens,
            accumulated_total_tokens,
            accumulated_input_tokens,
            accumulated_output_tokens
        FROM sessions
        WHERE model_config_json IS NOT NULL
          AND TRIM(model_config_json) != ''
    "#;

    let mut stmt = match conn.prepare(query) {
        Ok(s) => s,
        Err(err) => {
            warn!(
                db_path = %db_path.display(),
                error = %err,
                "Failed to prepare Goose session query"
            );
            return Vec::new();
        }
    };

    let rows = match stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, Option<i64>>(4)?,
            row.get::<_, Option<i64>>(5)?,
            row.get::<_, Option<i64>>(6)?,
            row.get::<_, Option<i64>>(7)?,
            row.get::<_, Option<i64>>(8)?,
            row.get::<_, Option<i64>>(9)?,
        ))
    }) {
        Ok(r) => r,
        Err(err) => {
            warn!(
                db_path = %db_path.display(),
                error = %err,
                "Failed to execute Goose session query"
            );
            return Vec::new();
        }
    };

    rows.filter_map(|row| match row {
        Ok(row) => Some(row),
        Err(err) => {
            warn!(
                db_path = %db_path.display(),
                error = %err,
                "Failed to decode Goose session row"
            );
            None
        }
    })
    .filter_map(
        |(
            session_id,
            model_config_json,
            provider_name,
            created_at,
            total_tokens,
            input_tokens,
            output_tokens,
            accumulated_total_tokens,
            accumulated_input_tokens,
            accumulated_output_tokens,
        )| {
            let model_config = model_config_json.as_ref()?;
            let model_id = parse_model_config(model_config)?;

            let created_at_ts = parse_created_at(&created_at);

            let input = accumulated_input_tokens
                .or(input_tokens)
                .unwrap_or(0)
                .max(0);
            let output = accumulated_output_tokens
                .or(output_tokens)
                .unwrap_or(0)
                .max(0);
            let total = accumulated_total_tokens
                .or(total_tokens)
                .unwrap_or(0)
                .max(0);

            if input == 0 && output == 0 && total == 0 {
                return None;
            }

            let provider = resolved_provider(provider_name, &model_id);
            let mut msg = UnifiedMessage::new(
                "goose",
                model_id,
                provider,
                session_id.clone(),
                timestamp_secs_to_ms(created_at_ts),
                TokenBreakdown {
                    input,
                    output,
                    cache_read: 0,
                    cache_write: 0,
                    reasoning: if total > input + output {
                        (total - input - output).max(0)
                    } else {
                        0
                    },
                },
                0.0,
            );
            msg.dedup_key = Some(session_id);
            Some(msg)
        },
    )
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_model_config_valid() {
        let json = r#"{"model_name":"claude-sonnet-4-20250514","context_limit":200000}"#;
        assert_eq!(
            parse_model_config(json),
            Some("claude-sonnet-4-20250514".to_string())
        );
    }

    #[test]
    fn test_parse_model_config_empty_name() {
        let json = r#"{"model_name":"  ","context_limit":200000}"#;
        assert_eq!(parse_model_config(json), None);
    }

    #[test]
    fn test_parse_model_config_invalid_json() {
        assert_eq!(parse_model_config("not json"), None);
    }

    #[test]
    fn test_timestamp_secs_to_ms() {
        assert_eq!(timestamp_secs_to_ms(1_700_000_000.0), 1_700_000_000_000);
        assert_eq!(timestamp_secs_to_ms(1_700_000_000_000.0), 1_700_000_000_000);
    }

    #[test]
    fn test_parse_created_at_rfc3339() {
        let ts = parse_created_at("2026-04-14T16:18:53Z");
        assert!(ts > 0.0);
    }

    #[test]
    fn test_parse_created_at_sqlite_timestamp() {
        let ts = parse_created_at("2026-04-14 16:18:53");
        assert!(ts > 0.0);
        let expected =
            chrono::NaiveDateTime::parse_from_str("2026-04-14 16:18:53", "%Y-%m-%d %H:%M:%S")
                .unwrap()
                .and_utc()
                .timestamp_millis() as f64;
        assert_eq!(ts, expected);
    }

    #[test]
    fn test_parse_created_at_date_only() {
        let ts = parse_created_at("2026-04-14");
        assert!(ts > 0.0);
    }

    #[test]
    fn test_parse_created_at_invalid() {
        assert_eq!(parse_created_at("not a date"), 0.0);
    }
}
