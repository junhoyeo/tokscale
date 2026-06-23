//! Mux (coder/mux) session parser
//!
//! Parses session-usage.json files from ~/.mux/sessions/<workspaceId>/session-usage.json

use super::utils::{file_modified_timestamp_ms, read_file_or_none};
use super::UnifiedMessage;
use crate::TokenBreakdown;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Deserialize)]
pub struct MuxSessionUsage {
    #[allow(dead_code)]
    pub version: Option<u32>,
    #[serde(rename = "byModel")]
    pub by_model: Option<HashMap<String, MuxModelUsage>>,
    #[serde(rename = "lastRequest")]
    pub last_request: Option<MuxLastRequest>,
}

#[derive(Debug, Deserialize)]
pub struct MuxModelUsage {
    pub input: Option<MuxTokenBucket>,
    pub cached: Option<MuxTokenBucket>,
    #[serde(rename = "cacheCreate")]
    pub cache_create: Option<MuxTokenBucket>,
    pub output: Option<MuxTokenBucket>,
    pub reasoning: Option<MuxTokenBucket>,
}

#[derive(Debug, Deserialize)]
pub struct MuxTokenBucket {
    pub tokens: Option<i64>,
    pub cost_usd: Option<f64>,
}

#[derive(Debug, Deserialize)]
pub struct MuxLastRequest {
    #[allow(dead_code)]
    pub model: Option<String>,
    pub timestamp: Option<i64>,
}

/// Parse a mux session-usage.json file.
/// Returns one UnifiedMessage per model entry in byModel.
pub fn parse_mux_file(path: &Path) -> Vec<UnifiedMessage> {
    let Some(data) = read_file_or_none(path) else {
        return vec![];
    };

    let usage: MuxSessionUsage = match serde_json::from_slice(&data) {
        Ok(u) => u,
        Err(_) => return vec![],
    };

    let timestamp = usage
        .last_request
        .as_ref()
        .and_then(|lr| lr.timestamp)
        .unwrap_or_else(|| file_modified_timestamp_ms(path));

    let session_id = path
        .parent()
        .and_then(|p| p.file_name())
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();

    let by_model = match usage.by_model {
        Some(m) => m,
        None => return vec![],
    };

    by_model
        .into_iter()
        .filter_map(|(model_key, model_usage)| {
            let tokens =
                |b: &Option<MuxTokenBucket>| b.as_ref().and_then(|b| b.tokens).unwrap_or(0).max(0);
            let cost =
                |b: &Option<MuxTokenBucket>| b.as_ref().and_then(|b| b.cost_usd).unwrap_or(0.0);
            let input = tokens(&model_usage.input);
            let cached = tokens(&model_usage.cached);
            let cache_create = tokens(&model_usage.cache_create);
            let output = tokens(&model_usage.output);
            let reasoning = tokens(&model_usage.reasoning);
            let source_cost = cost(&model_usage.input)
                + cost(&model_usage.cached)
                + cost(&model_usage.cache_create)
                + cost(&model_usage.output)
                + cost(&model_usage.reasoning);

            if input == 0 && cached == 0 && cache_create == 0 && output == 0 && reasoning == 0 {
                return None;
            }

            // Strip "provider:" prefix for model ID (e.g., "anthropic:claude-opus-4-6" -> "claude-opus-4-6")
            let (provider, model_id) = if model_key.contains(':') {
                let mut parts = model_key.splitn(2, ':');
                let p = parts.next().unwrap_or("").to_string();
                let m = parts.next().unwrap_or(&model_key).to_string();
                (p, m)
            } else {
                (String::new(), model_key)
            };

            Some(UnifiedMessage::new(
                "mux",
                model_id,
                provider,
                session_id.clone(),
                timestamp,
                TokenBreakdown {
                    input,
                    output,
                    cache_read: cached,
                    cache_write: cache_create,
                    reasoning,
                },
                source_cost,
            ))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn write_temp_json(content: &str) -> NamedTempFile {
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(content.as_bytes()).unwrap();
        f.flush().unwrap();
        f
    }

    #[test]
    fn test_parse_valid_session_usage() {
        let json = r#"{
            "version": 1,
            "byModel": {
                "anthropic:claude-opus-4-6": {
                    "input": { "tokens": 100, "cost_usd": 0.01 },
                    "cached": { "tokens": 5000, "cost_usd": 0.05 },
                    "cacheCreate": { "tokens": 200, "cost_usd": 0.02 },
                    "output": { "tokens": 300, "cost_usd": 0.03 },
                    "reasoning": { "tokens": 0, "cost_usd": 0 }
                },
                "openai:gpt-4o": {
                    "input": { "tokens": 50, "cost_usd": 0.005 },
                    "cached": { "tokens": 0, "cost_usd": 0 },
                    "cacheCreate": { "tokens": 0, "cost_usd": 0 },
                    "output": { "tokens": 150, "cost_usd": 0.015 },
                    "reasoning": { "tokens": 0, "cost_usd": 0 }
                }
            },
            "lastRequest": {
                "model": "anthropic:claude-opus-4-6",
                "timestamp": 1700000000000
            }
        }"#;
        let f = write_temp_json(json);
        let msgs = parse_mux_file(f.path());
        assert_eq!(msgs.len(), 2);

        // Find the claude message
        let claude = msgs
            .iter()
            .find(|m| m.model_id == "claude-opus-4-6")
            .unwrap();
        assert_eq!(claude.client, "mux");
        assert_eq!(claude.provider_id, "anthropic");
        assert_eq!(claude.tokens.input, 100);
        assert_eq!(claude.tokens.cache_read, 5000);
        assert_eq!(claude.tokens.cache_write, 200);
        assert_eq!(claude.tokens.output, 300);
        assert_eq!(claude.tokens.reasoning, 0);
        assert_eq!(claude.timestamp, 1700000000000);

        let gpt = msgs.iter().find(|m| m.model_id == "gpt-4o").unwrap();
        assert_eq!(gpt.provider_id, "openai");
        assert_eq!(gpt.tokens.input, 50);
        assert_eq!(gpt.tokens.output, 150);
    }

    #[test]
    fn test_parse_empty_by_model() {
        let json = r#"{ "version": 1, "byModel": {} }"#;
        let f = write_temp_json(json);
        let msgs = parse_mux_file(f.path());
        assert!(msgs.is_empty());
    }

    #[test]
    fn test_parse_missing_by_model() {
        let json = r#"{ "version": 1 }"#;
        let f = write_temp_json(json);
        let msgs = parse_mux_file(f.path());
        assert!(msgs.is_empty());
    }

    #[test]
    fn test_zero_token_entries_filtered() {
        let json = r#"{
            "version": 1,
            "byModel": {
                "anthropic:claude-opus-4-6": {
                    "input": { "tokens": 0, "cost_usd": 0 },
                    "cached": { "tokens": 0, "cost_usd": 0 },
                    "cacheCreate": { "tokens": 0, "cost_usd": 0 },
                    "output": { "tokens": 0, "cost_usd": 0 },
                    "reasoning": { "tokens": 0, "cost_usd": 0 }
                }
            },
            "lastRequest": { "model": "anthropic:claude-opus-4-6", "timestamp": 1700000000000 }
        }"#;
        let f = write_temp_json(json);
        let msgs = parse_mux_file(f.path());
        assert!(msgs.is_empty());
    }

    #[test]
    fn test_model_without_provider_prefix() {
        let json = r#"{
            "version": 1,
            "byModel": {
                "claude-opus-4-6": {
                    "input": { "tokens": 100 },
                    "output": { "tokens": 200 }
                }
            },
            "lastRequest": { "timestamp": 1700000000000 }
        }"#;
        let f = write_temp_json(json);
        let msgs = parse_mux_file(f.path());
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].model_id, "claude-opus-4-6");
        assert_eq!(msgs[0].provider_id, "");
    }

    #[test]
    fn test_invalid_json() {
        let f = write_temp_json("not json at all");
        let msgs = parse_mux_file(f.path());
        assert!(msgs.is_empty());
    }

    #[test]
    fn test_nonexistent_file() {
        let msgs = parse_mux_file(Path::new("/nonexistent/path/session-usage.json"));
        assert!(msgs.is_empty());
    }

    #[test]
    fn test_negative_tokens_clamped() {
        let json = r#"{
            "version": 1,
            "byModel": {
                "anthropic:claude-opus-4-6": {
                    "input": { "tokens": -50, "cost_usd": 0.01 },
                    "output": { "tokens": 100, "cost_usd": 0.02 }
                }
            },
            "lastRequest": { "timestamp": 1700000000000 }
        }"#;
        let f = write_temp_json(json);
        let msgs = parse_mux_file(f.path());
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].tokens.input, 0);
        assert_eq!(msgs[0].tokens.output, 100);
    }

    #[test]
    fn test_source_cost_summed() {
        let json = r#"{
            "version": 1,
            "byModel": {
                "anthropic:claude-opus-4-6": {
                    "input": { "tokens": 100, "cost_usd": 0.01 },
                    "cached": { "tokens": 200, "cost_usd": 0.02 },
                    "cacheCreate": { "tokens": 50, "cost_usd": 0.005 },
                    "output": { "tokens": 300, "cost_usd": 0.03 },
                    "reasoning": { "tokens": 0, "cost_usd": 0 }
                }
            },
            "lastRequest": { "timestamp": 1700000000000 }
        }"#;
        let f = write_temp_json(json);
        let msgs = parse_mux_file(f.path());
        assert_eq!(msgs.len(), 1);
        let expected_cost = 0.01 + 0.02 + 0.005 + 0.03;
        assert!((msgs[0].cost - expected_cost).abs() < 1e-10);
    }

    #[test]
    fn test_multi_colon_model_key() {
        let json = r#"{
            "version": 1,
            "byModel": {
                "provider:sub:model-name": {
                    "input": { "tokens": 100 },
                    "output": { "tokens": 200 }
                }
            },
            "lastRequest": { "timestamp": 1700000000000 }
        }"#;
        let f = write_temp_json(json);
        let msgs = parse_mux_file(f.path());
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].provider_id, "provider");
        assert_eq!(msgs[0].model_id, "sub:model-name");
    }
}
