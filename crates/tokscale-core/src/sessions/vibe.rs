//! Mistral Vibe session parser
//!
//! Mistral Vibe (Mistral AI's terminal coding agent) stores session logs under
//! `~/.vibe/logs/session/` (or `$VIBE_HOME/logs/session/`).
//!
//! Directory layout (Vibe 2.x):
//!   ~/.vibe/logs/session/session_YYYYMMDD_HHMMSS_<short_id>/
//!     ├── meta.json
//!     └── messages.jsonl
//!
//! Legacy Vibe 1.x flat format:
//!   ~/.vibe/logs/session/session_*.json
//!
//! Usage statistics are recorded in `meta.json` (or the top-level `metadata`
//! key in legacy session files) under `stats`:
//! - `session_prompt_tokens`: overall prompt tokens (includes cached prompt tokens)
//! - `session_completion_tokens`: completion / output tokens
//! - `session_cached_tokens`: cached prompt tokens

use super::utils::{file_modified_timestamp_ms, parse_timestamp_value};
use super::{normalize_workspace_key, workspace_label_from_key, UnifiedMessage};
use crate::{provider_identity, TokenBreakdown};
use serde_json::Value;
use std::path::Path;

pub fn parse_vibe_file(path: &Path) -> Vec<UnifiedMessage> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    if content.trim().is_empty() {
        return Vec::new();
    }
    let Ok(root) = serde_json::from_str::<Value>(&content) else {
        return Vec::new();
    };

    // Vibe 1.x flat files wrap metadata under a "metadata" key;
    // Vibe 2.x meta.json has metadata at the root.
    let meta = if let Some(sub_meta) = root.get("metadata").filter(|v| v.is_object()) {
        sub_meta
    } else {
        &root
    };

    let Some(stats) = meta.get("stats").filter(|v| v.is_object()) else {
        return Vec::new();
    };

    let prompt_tokens = stats
        .get("session_prompt_tokens")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let completion_tokens = stats
        .get("session_completion_tokens")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let cached_tokens = stats
        .get("session_cached_tokens")
        .and_then(Value::as_i64)
        .unwrap_or(0);

    let input = prompt_tokens.saturating_sub(cached_tokens).max(0);
    let output = completion_tokens.max(0);
    let cache_read = cached_tokens.max(0);

    let tokens = TokenBreakdown {
        input,
        output,
        cache_read,
        ..Default::default()
    };

    if tokens.total() == 0 {
        return Vec::new();
    }

    let timestamp = meta
        .get("end_time")
        .and_then(parse_timestamp_value)
        .or_else(|| meta.get("start_time").and_then(parse_timestamp_value))
        .filter(|&ts| ts > 0)
        .unwrap_or_else(|| file_modified_timestamp_ms(path));

    let session_id = meta
        .get("session_id")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(String::from)
        .unwrap_or_else(|| {
            if path.file_name().and_then(|f| f.to_str()) == Some("meta.json") {
                path.parent()
                    .and_then(|p| p.file_name())
                    .and_then(|f| f.to_str())
                    .unwrap_or("unknown")
                    .to_string()
            } else {
                path.file_stem()
                    .and_then(|f| f.to_str())
                    .unwrap_or("unknown")
                    .to_string()
            }
        });

    let raw_model = meta
        .pointer("/config/active_model")
        .and_then(Value::as_str)
        .or_else(|| meta.get("active_model").and_then(Value::as_str))
        .or_else(|| meta.get("model").and_then(Value::as_str))
        .unwrap_or("mistral-large-latest");

    let model = raw_model.trim();
    let inferred = provider_identity::inferred_provider_from_model(model).unwrap_or("mistral");
    let canonical_prov = provider_identity::canonical_provider(inferred);
    let provider = canonical_prov.as_deref().unwrap_or(inferred);

    let dedup_key = format!("vibe:{session_id}");

    let mut message = UnifiedMessage::new_with_dedup(
        "vibe",
        model,
        provider,
        session_id,
        timestamp,
        tokens,
        0.0,
        Some(dedup_key),
    );

    let origin_dir = meta
        .get("origin_directory")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|d| !d.is_empty())
        .or_else(|| {
            meta.pointer("/environment/working_directory")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|d| !d.is_empty())
        });

    if let Some(dir) = origin_dir {
        let workspace_key = normalize_workspace_key(dir);
        let workspace_label = workspace_key.as_deref().and_then(workspace_label_from_key);
        if workspace_key.is_some() {
            message.set_workspace(workspace_key, workspace_label);
        }
    }

    if let Some(title) = meta.get("title").and_then(Value::as_str) {
        let trimmed = title.trim();
        if !trimmed.is_empty() {
            message.session_title = Some(trimmed.to_string());
        }
    }

    if let Some(steps) = stats
        .get("steps")
        .and_then(Value::as_i64)
        .filter(|&s| s > 0)
    {
        message.message_count = steps.min(i32::MAX as i64) as i32;
    } else if let Some(total_msgs) = meta
        .get("total_messages")
        .and_then(Value::as_i64)
        .filter(|&m| m > 0)
    {
        message.message_count = total_msgs.min(i32::MAX as i64) as i32;
    }

    vec![message]
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_parse_vibe_v2_metadata() {
        let dir = tempdir().unwrap();
        let session_dir = dir.path().join("session_20261010_080000_abc123");
        std::fs::create_dir_all(&session_dir).unwrap();
        let meta_file = session_dir.join("meta.json");

        let json = r#"{
            "session_id": "019e1e27-6f8e-73cb-b0eb-78da684e5111",
            "start_time": "2026-10-10T08:00:00Z",
            "end_time": "2026-10-10T08:05:00Z",
            "title": "Refactor parser",
            "origin_directory": "/home/ubuntu/repo",
            "environment": {
                "working_directory": "/home/ubuntu/repo"
            },
            "config": {
                "active_model": "codestral-latest"
            },
            "stats": {
                "steps": 3,
                "session_prompt_tokens": 15000,
                "session_completion_tokens": 1200,
                "session_cached_tokens": 5000
            }
        }"#;
        std::fs::write(&meta_file, json).unwrap();

        let msgs = parse_vibe_file(&meta_file);
        assert_eq!(msgs.len(), 1);
        let msg = &msgs[0];
        assert_eq!(msg.client, "vibe");
        assert_eq!(msg.session_id, "019e1e27-6f8e-73cb-b0eb-78da684e5111");
        assert_eq!(msg.model_id, "codestral-latest");
        assert_eq!(msg.provider_id, "mistralai");
        assert_eq!(msg.tokens.input, 10000); // 15000 - 5000
        assert_eq!(msg.tokens.output, 1200);
        assert_eq!(msg.tokens.cache_read, 5000);
        assert_eq!(msg.message_count, 3);
        assert_eq!(msg.session_title.as_deref(), Some("Refactor parser"));
        assert_eq!(msg.workspace_key.as_deref(), Some("/home/ubuntu/repo"));
    }

    #[test]
    fn test_parse_vibe_v1_legacy_format() {
        let dir = tempdir().unwrap();
        let session_file = dir.path().join("session_20260901_010203.json");

        let json = r#"{
            "metadata": {
                "session_id": "legacy-session-1",
                "start_time": "2026-09-01T01:02:03Z",
                "active_model": "mistral-large-latest",
                "stats": {
                    "steps": 1,
                    "session_prompt_tokens": 8000,
                    "session_completion_tokens": 500,
                    "session_cached_tokens": 0
                }
            },
            "messages": []
        }"#;
        std::fs::write(&session_file, json).unwrap();

        let msgs = parse_vibe_file(&session_file);
        assert_eq!(msgs.len(), 1);
        let msg = &msgs[0];
        assert_eq!(msg.client, "vibe");
        assert_eq!(msg.session_id, "legacy-session-1");
        assert_eq!(msg.model_id, "mistral-large-latest");
        assert_eq!(msg.tokens.input, 8000);
        assert_eq!(msg.tokens.output, 500);
        assert_eq!(msg.tokens.cache_read, 0);
    }

    #[test]
    fn test_parse_vibe_skips_zero_usage() {
        let dir = tempdir().unwrap();
        let meta_file = dir.path().join("meta.json");

        let json = r#"{
            "session_id": "zero-session",
            "stats": {
                "session_prompt_tokens": 0,
                "session_completion_tokens": 0,
                "session_cached_tokens": 0
            }
        }"#;
        std::fs::write(&meta_file, json).unwrap();

        let msgs = parse_vibe_file(&meta_file);
        assert!(msgs.is_empty());
    }
}
