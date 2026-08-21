//! Goose session parser
//!
//! Parses session rows from Goose's SQLite sessions database:
//! - Primary: `~/.local/share/goose/sessions/sessions.db`
//! - macOS: `~/Library/Application Support/goose/sessions/sessions.db`
//! - Legacy Block/goose: `~/.local/share/Block/goose/sessions/sessions.db`
//! - Custom: `$GOOSE_PATH_ROOT/data/sessions/sessions.db`

use super::utils::{resolved_provider, sqlite_for_each_row, timestamp_secs_to_ms};
use super::UnifiedMessage;
use crate::TokenBreakdown;
use serde::Deserialize;
use std::path::Path;

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

    let mut messages = Vec::new();
    sqlite_for_each_row(db_path, query, Some("Goose session"), &mut |row| {
        let session_id: String = row.get(0)?;
        let model_config_json: Option<String> = row.get(1)?;
        let provider_name: Option<String> = row.get(2)?;
        let created_at: String = row.get(3)?;
        let total_tokens: Option<i64> = row.get(4)?;
        let input_tokens: Option<i64> = row.get(5)?;
        let output_tokens: Option<i64> = row.get(6)?;
        let accumulated_total_tokens: Option<i64> = row.get(7)?;
        let accumulated_input_tokens: Option<i64> = row.get(8)?;
        let accumulated_output_tokens: Option<i64> = row.get(9)?;

        let Some(model_config) = model_config_json.as_ref() else {
            return Ok(());
        };
        let Some(model_id) = parse_model_config(model_config) else {
            return Ok(());
        };

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
            return Ok(());
        }

        let provider = resolved_provider(provider_name, &model_id, "goose");
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
                // INFERRED, not a real field: Goose's schema has no reasoning
                // token column. We heuristically attribute any gap between the
                // reported total and (input + output) to reasoning. This is a
                // best-effort estimate, not a measured count.
                reasoning: if total > input + output {
                    (total - input - output).max(0)
                } else {
                    0
                },
            },
            0.0,
        );
        msg.dedup_key = Some(session_id);
        messages.push(msg);
        Ok(())
    });

    messages
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_goose_sqlite_returns_empty_for_missing_database() {
        let temp_dir = tempfile::tempdir().unwrap();
        let missing_db = temp_dir.path().join("missing.db");

        assert!(parse_goose_sqlite(&missing_db).is_empty());
    }

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
