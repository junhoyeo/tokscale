//! Cursor IDE session parser
//!
//! Parses CSV files from the Cursor usage export API.
//! CSV files are cached locally at ~/.config/tokscale/cursor-cache/usage.csv
//!
//! CSV Format (actual from API):
//! Date,Kind,Model,Max Mode,Input (w/ Cache Write),Input (w/o Cache Write),Cache Read,Output Tokens,Total Tokens,Cost

use super::UnifiedMessage;
use crate::TokenBreakdown;
use std::path::Path;

/// Provider inference from model name
fn infer_provider(model: &str) -> &'static str {
    let lower = model.to_lowercase();

    if lower.contains("claude")
        || lower.contains("sonnet")
        || lower.contains("opus")
        || lower.contains("haiku")
    {
        "anthropic"
    } else if lower.contains("gpt") || lower.contains("o1") || lower.contains("o3") {
        "openai"
    } else if lower.contains("gemini") {
        "google"
    } else if lower.contains("deepseek") {
        "deepseek"
    } else if lower.contains("llama") || lower.contains("mixtral") {
        "meta"
    } else {
        "cursor"
    }
}

/// Parse a cost string like "$0.50" or "0.50" to f64
/// Returns 0.0 for empty strings, NaN values, or invalid formats
fn parse_cost(cost_str: &str) -> f64 {
    let cleaned = cost_str.replace(['$', ','], "");
    let trimmed = cleaned.trim();

    // Handle empty or NaN values
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("nan") {
        return 0.0;
    }

    trimmed.parse().unwrap_or(0.0)
}

/// Parse a Cursor usage CSV file
///
/// Handles both formats:
/// - New: Date,Kind,Model,Max Mode,Input (w/ Cache Write),Input (w/o Cache Write),Cache Read,Output Tokens,Total Tokens,Cost
/// - Old: Date,Model,Input (w/ Cache Write),Input (w/o Cache Write),Cache Read,Output Tokens,Total Tokens,Cost,Cost to you
pub fn parse_cursor_file(path: &Path) -> Vec<UnifiedMessage> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return vec![],
    };

    let mut messages = Vec::new();
    let mut lines = content.lines();

    // Parse header line to determine column indices
    let header = match lines.next() {
        Some(h) => h,
        None => return vec![],
    };

    // Verify this is a valid Cursor CSV
    if !header.contains("Date") || !header.contains("Model") {
        return vec![];
    }

    // Detect format by checking for "Kind" column
    let header_fields: Vec<&str> = parse_csv_line(header);
    let has_kind_column = header_fields.iter().any(|f| f.trim() == "Kind");

    // Column indices based on format
    let (
        model_idx,
        input_cache_write_idx,
        input_no_cache_idx,
        cache_read_idx,
        output_idx,
        cost_idx,
    ) = if has_kind_column {
        // New format: Date,Kind,Model,Max Mode,Input (w/ Cache Write),...
        (2, 4, 5, 6, 7, 9)
    } else {
        // Old format: Date,Model,Input (w/ Cache Write),...
        (1, 2, 3, 4, 5, 7)
    };

    for line in lines {
        if line.trim().is_empty() {
            continue;
        }

        // Parse CSV line (simple parsing, handles quoted fields)
        let fields: Vec<&str> = parse_csv_line(line);

        // Need at least enough columns for the format
        let min_fields = cost_idx + 1;
        if fields.len() < min_fields {
            continue;
        }

        let date_str = fields[0].trim().trim_matches('"');
        let model = fields[model_idx].trim().trim_matches('"');
        let input_with_cache_write: i64 = fields[input_cache_write_idx]
            .trim()
            .trim_matches('"')
            .parse()
            .unwrap_or(0);
        let input_without_cache_write: i64 = fields[input_no_cache_idx]
            .trim()
            .trim_matches('"')
            .parse()
            .unwrap_or(0);
        let cache_read: i64 = fields[cache_read_idx]
            .trim()
            .trim_matches('"')
            .parse()
            .unwrap_or(0);
        let output_tokens: i64 = fields[output_idx]
            .trim()
            .trim_matches('"')
            .parse()
            .unwrap_or(0);
        let cost_str = fields[cost_idx].trim().trim_matches('"');
        let cost = parse_cost(cost_str);

        // Skip empty or errored entries
        if model.is_empty() {
            continue;
        }

        // Parse timestamp from date string
        let timestamp = parse_date_to_timestamp(date_str);
        if timestamp == 0 {
            continue;
        }

        // Cache write = input_with_cache_write - input_without_cache_write
        let cache_write = (input_with_cache_write - input_without_cache_write).max(0);
        // Input tokens = input_without_cache_write
        let input = input_without_cache_write;

        messages.push(UnifiedMessage::new(
            "cursor",
            model,
            infer_provider(model),
            date_str.to_string(),
            timestamp,
            TokenBreakdown {
                input,
                output: output_tokens,
                cache_read,
                cache_write,
                reasoning: 0,
            },
            cost,
        ));
    }

    messages
}

/// Simple CSV line parser that handles quoted fields
fn parse_csv_line(line: &str) -> Vec<&str> {
    let mut fields = Vec::new();
    let mut start = 0;
    let mut in_quotes = false;
    let bytes = line.as_bytes();

    for (i, &byte) in bytes.iter().enumerate() {
        match byte {
            b'"' => in_quotes = !in_quotes,
            b',' if !in_quotes => {
                fields.push(&line[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }

    // Add the last field
    if start <= line.len() {
        fields.push(&line[start..]);
    }

    fields
}

/// Parse a date string to Unix milliseconds timestamp
fn parse_date_to_timestamp(date_str: &str) -> i64 {
    use chrono::{NaiveDate, NaiveDateTime, TimeZone, Utc};

    // Try ISO 8601 format with milliseconds: "2025-02-05T12:00:00.123Z"
    if let Ok(dt) = NaiveDateTime::parse_from_str(date_str, "%Y-%m-%dT%H:%M:%S%.3fZ") {
        return Utc.from_utc_datetime(&dt).timestamp_millis();
    }

    // Try ISO 8601 format with time: "2025-02-05T12:00:00Z"
    if let Ok(dt) = NaiveDateTime::parse_from_str(date_str, "%Y-%m-%dT%H:%M:%SZ") {
        return Utc.from_utc_datetime(&dt).timestamp_millis();
    }

    // Try ISO 8601 format with milliseconds without Z: "2025-02-05T12:00:00.123"
    if let Ok(dt) = NaiveDateTime::parse_from_str(date_str, "%Y-%m-%dT%H:%M:%S%.3f") {
        return Utc.from_utc_datetime(&dt).timestamp_millis();
    }

    // Try ISO 8601 format with time without Z: "2025-02-05T12:00:00"
    if let Ok(dt) = NaiveDateTime::parse_from_str(date_str, "%Y-%m-%dT%H:%M:%S") {
        return Utc.from_utc_datetime(&dt).timestamp_millis();
    }

    // Try date only format: "2025-02-05"
    if let Ok(date) = NaiveDate::parse_from_str(date_str, "%Y-%m-%d") {
        let dt = date.and_hms_opt(12, 0, 0).unwrap(); // Noon UTC
        return Utc.from_utc_datetime(&dt).timestamp_millis();
    }

    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_infer_provider() {
        assert_eq!(infer_provider("claude-3-sonnet"), "anthropic");
        assert_eq!(infer_provider("gpt-4o"), "openai");
        assert_eq!(infer_provider("gemini-pro"), "google");
        assert_eq!(infer_provider("deepseek-coder"), "deepseek");
        assert_eq!(infer_provider("llama-3"), "meta");
        assert_eq!(infer_provider("unknown-model"), "cursor");
    }

    #[test]
    fn test_parse_cost() {
        assert_eq!(parse_cost("$0.50"), 0.50);
        assert_eq!(parse_cost("0.50"), 0.50);
        assert_eq!(parse_cost("$1,234.56"), 1234.56);
        assert_eq!(parse_cost(""), 0.0);
        assert_eq!(parse_cost("NaN"), 0.0);
        assert_eq!(parse_cost("nan"), 0.0);
        assert_eq!(parse_cost("  "), 0.0);
    }

    #[test]
    fn test_parse_csv_line() {
        let line = "2025-02-01,gpt-4o,10,5,0,15,30,$0.10,$0.10";
        let fields = parse_csv_line(line);
        assert_eq!(fields.len(), 9);
        assert_eq!(fields[0], "2025-02-01");
        assert_eq!(fields[1], "gpt-4o");
        assert_eq!(fields[8], "$0.10");
    }

    #[test]
    fn test_parse_date_to_timestamp() {
        // ISO with milliseconds and Z (new Cursor format)
        let ts = parse_date_to_timestamp("2025-11-13T18:36:05.846Z");
        assert!(ts > 0);

        // ISO with Z
        let ts = parse_date_to_timestamp("2025-02-05T12:00:00Z");
        assert!(ts > 0);

        // Date only
        let ts = parse_date_to_timestamp("2025-02-05");
        assert!(ts > 0);

        // Invalid
        let ts = parse_date_to_timestamp("invalid");
        assert_eq!(ts, 0);
    }

    #[test]
    fn test_parse_cursor_csv_sample_old_format() {
        let csv = "Date,Model,Input (w/ Cache Write),Input (w/o Cache Write),Cache Read,Output Tokens,Total Tokens,Cost,Cost to you
2025-02-01,gpt-4o,10,5,0,15,30,$0.10,$0.10
2025-02-02,gpt-4o-mini,0,0,0,5,5,$0.05,$0.05";

        let temp_dir = tempfile::TempDir::new().unwrap();
        let file_path = temp_dir.path().join("usage.csv");
        std::fs::write(&file_path, csv).unwrap();

        let messages = parse_cursor_file(&file_path);
        assert_eq!(messages.len(), 2);

        assert_eq!(messages[0].source, "cursor");
        assert_eq!(messages[0].model_id, "gpt-4o");
        assert_eq!(messages[0].provider_id, "openai");
        assert_eq!(messages[0].tokens.input, 5);
        assert_eq!(messages[0].tokens.output, 15);
        assert_eq!(messages[0].tokens.cache_write, 5); // 10 - 5
        assert!((messages[0].cost - 0.10).abs() < 0.001);

        assert_eq!(messages[1].model_id, "gpt-4o-mini");
    }

    #[test]
    fn test_parse_cursor_csv_sample_new_format() {
        // Real format from Cursor API
        let csv = r#"Date,Kind,Model,Max Mode,Input (w/ Cache Write),Input (w/o Cache Write),Cache Read,Output Tokens,Total Tokens,Cost
"2025-11-13T18:36:05.846Z","Included","auto","No","28342","775","105891","21282","156290","0.19"
"2025-11-13T13:35:04.658Z","On-Demand","gpt-5-codex","No","0","8263","66964","1612","76839","0.03""#;

        let temp_dir = tempfile::TempDir::new().unwrap();
        let file_path = temp_dir.path().join("usage.csv");
        std::fs::write(&file_path, csv).unwrap();

        let messages = parse_cursor_file(&file_path);
        assert_eq!(messages.len(), 2);

        // First message: auto model
        assert_eq!(messages[0].source, "cursor");
        assert_eq!(messages[0].model_id, "auto");
        assert_eq!(messages[0].provider_id, "cursor"); // unknown model -> cursor
        assert_eq!(messages[0].tokens.input, 775);
        assert_eq!(messages[0].tokens.output, 21282);
        assert_eq!(messages[0].tokens.cache_read, 105891);
        assert_eq!(messages[0].tokens.cache_write, 28342 - 775); // 27567
        assert!((messages[0].cost - 0.19).abs() < 0.001);

        // Second message: gpt-5-codex
        assert_eq!(messages[1].model_id, "gpt-5-codex");
        assert_eq!(messages[1].provider_id, "openai"); // gpt -> openai
        assert_eq!(messages[1].tokens.input, 8263);
        assert_eq!(messages[1].tokens.cache_read, 66964);
    }
}
