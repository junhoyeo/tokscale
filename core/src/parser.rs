//! SIMD-accelerated JSON parser
//!
//! Uses simd-json for fast JSON parsing with SIMD instructions.

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::Path;

/// Parse a JSON file using SIMD-accelerated parsing
pub fn parse_json_file<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, ParseError> {
    let mut data = fs::read(path).map_err(|e| ParseError::IoError(e.to_string()))?;

    simd_json::from_slice(&mut data).map_err(|e| ParseError::JsonError(e.to_string()))
}

/// Parse a JSONL file (one JSON object per line)
pub fn parse_jsonl_file<T, F>(path: &Path, mut process: F) -> Result<(), ParseError>
where
    T: serde::de::DeserializeOwned,
    F: FnMut(T),
{
    let file = fs::File::open(path).map_err(|e| ParseError::IoError(e.to_string()))?;
    let reader = BufReader::new(file);

    for line in reader.lines() {
        let line = line.map_err(|e| ParseError::IoError(e.to_string()))?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let mut bytes = trimmed.as_bytes().to_vec();
        match simd_json::from_slice::<T>(&mut bytes) {
            Ok(value) => process(value),
            Err(_) => continue, // Skip malformed lines (match TypeScript behavior)
        }
    }

    Ok(())
}

/// Parse error types
#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("IO error: {0}")]
    IoError(String),

    #[error("JSON parse error: {0}")]
    JsonError(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use std::fs::File;
    use std::io::Write;
    use tempfile::TempDir;

    #[derive(Debug, Deserialize, PartialEq)]
    struct TestStruct {
        name: String,
        value: i32,
    }

    #[derive(Debug, Deserialize, PartialEq)]
    struct NestedStruct {
        id: String,
        data: InnerData,
    }

    #[derive(Debug, Deserialize, PartialEq)]
    struct InnerData {
        tokens: i64,
        cost: f64,
    }

    #[test]
    fn test_parse_json_file_simple() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("test.json");

        let mut file = File::create(&file_path).unwrap();
        file.write_all(br#"{"name": "test", "value": 42}"#).unwrap();

        let result: TestStruct = parse_json_file(&file_path).unwrap();
        assert_eq!(result.name, "test");
        assert_eq!(result.value, 42);
    }

    #[test]
    fn test_parse_json_file_nested() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("nested.json");

        let mut file = File::create(&file_path).unwrap();
        file.write_all(br#"{"id": "session-001", "data": {"tokens": 1000, "cost": 0.05}}"#)
            .unwrap();

        let result: NestedStruct = parse_json_file(&file_path).unwrap();
        assert_eq!(result.id, "session-001");
        assert_eq!(result.data.tokens, 1000);
        assert_eq!(result.data.cost, 0.05);
    }

    #[test]
    fn test_parse_json_file_unicode() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("unicode.json");

        let mut file = File::create(&file_path).unwrap();
        file.write_all(r#"{"name": "테스트", "value": 100}"#.as_bytes())
            .unwrap();

        let result: TestStruct = parse_json_file(&file_path).unwrap();
        assert_eq!(result.name, "테스트");
        assert_eq!(result.value, 100);
    }

    #[test]
    fn test_parse_json_file_not_found() {
        let result: Result<TestStruct, _> = parse_json_file(Path::new("/nonexistent/file.json"));
        assert!(matches!(result, Err(ParseError::IoError(_))));
    }

    #[test]
    fn test_parse_json_file_invalid_json() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("invalid.json");

        let mut file = File::create(&file_path).unwrap();
        file.write_all(b"not valid json {").unwrap();

        let result: Result<TestStruct, _> = parse_json_file(&file_path);
        assert!(matches!(result, Err(ParseError::JsonError(_))));
    }

    #[test]
    fn test_parse_json_file_empty() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("empty.json");

        File::create(&file_path).unwrap();

        let result: Result<TestStruct, _> = parse_json_file(&file_path);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_jsonl_file_basic() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("test.jsonl");

        let mut file = File::create(&file_path).unwrap();
        writeln!(file, r#"{{"name": "first", "value": 1}}"#).unwrap();
        writeln!(file, r#"{{"name": "second", "value": 2}}"#).unwrap();
        writeln!(file, r#"{{"name": "third", "value": 3}}"#).unwrap();

        let mut results: Vec<TestStruct> = Vec::new();
        parse_jsonl_file(&file_path, |item: TestStruct| {
            results.push(item);
        })
        .unwrap();

        assert_eq!(results.len(), 3);
        assert_eq!(results[0].name, "first");
        assert_eq!(results[1].value, 2);
        assert_eq!(results[2].name, "third");
    }

    #[test]
    fn test_parse_jsonl_file_with_empty_lines() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("with_empty.jsonl");

        let mut file = File::create(&file_path).unwrap();
        writeln!(file, r#"{{"name": "a", "value": 1}}"#).unwrap();
        writeln!(file, "").unwrap(); // Empty line
        writeln!(file, "   ").unwrap(); // Whitespace only
        writeln!(file, r#"{{"name": "b", "value": 2}}"#).unwrap();

        let mut results: Vec<TestStruct> = Vec::new();
        parse_jsonl_file(&file_path, |item: TestStruct| {
            results.push(item);
        })
        .unwrap();

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].name, "a");
        assert_eq!(results[1].name, "b");
    }

    #[test]
    fn test_parse_jsonl_file_with_malformed_lines() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("malformed.jsonl");

        let mut file = File::create(&file_path).unwrap();
        writeln!(file, r#"{{"name": "good", "value": 1}}"#).unwrap();
        writeln!(file, "not valid json").unwrap(); // Should be skipped
        writeln!(file, r#"{{"incomplete"#).unwrap(); // Should be skipped
        writeln!(file, r#"{{"name": "also good", "value": 2}}"#).unwrap();

        let mut results: Vec<TestStruct> = Vec::new();
        parse_jsonl_file(&file_path, |item: TestStruct| {
            results.push(item);
        })
        .unwrap();

        // Only valid lines should be parsed
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].name, "good");
        assert_eq!(results[1].name, "also good");
    }

    #[test]
    fn test_parse_jsonl_file_empty() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("empty.jsonl");

        File::create(&file_path).unwrap();

        let mut count = 0;
        parse_jsonl_file(&file_path, |_: TestStruct| {
            count += 1;
        })
        .unwrap();

        assert_eq!(count, 0);
    }

    #[test]
    fn test_parse_jsonl_file_not_found() {
        let result = parse_jsonl_file(Path::new("/nonexistent/file.jsonl"), |_: TestStruct| {});
        assert!(matches!(result, Err(ParseError::IoError(_))));
    }

    #[test]
    fn test_parse_jsonl_large_file() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("large.jsonl");

        let mut file = File::create(&file_path).unwrap();
        for i in 0..1000 {
            writeln!(file, r#"{{"name": "item-{}", "value": {}}}"#, i, i).unwrap();
        }

        let mut count = 0;
        parse_jsonl_file(&file_path, |_: TestStruct| {
            count += 1;
        })
        .unwrap();

        assert_eq!(count, 1000);
    }

    #[test]
    fn test_parse_error_display() {
        let io_error = ParseError::IoError("file not found".to_string());
        assert!(io_error.to_string().contains("IO error"));

        let json_error = ParseError::JsonError("unexpected token".to_string());
        assert!(json_error.to_string().contains("JSON parse error"));
    }
}
