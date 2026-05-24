//! Trae usage API response parser.
//!
//! Reads `trae-cache/sessions/*.json` — the raw JSON dumped
//! from the usage API — and converts each entry into a `UnifiedMessage`.
//!
//! The API already returns exact token counts, so this parser does not
//! go through `pricing/lookup`.

use super::UnifiedMessage;
use crate::TokenBreakdown;

/// Known mapping from Trae display names to tiktoken-style model ids.
/// Unknown names fall through to the raw `model_name` from the API
/// (mixed-case, space-separated).
fn normalize_trae_model(name: &str) -> String {
    match name {
        "GPT-5.4" => "gpt-5.4",
        "GPT-5.3-Codex" | "GPT-5.3 Codex" => "gpt-5.3-codex",
        "GPT-5.3" => "gpt-5.3",
        "GPT-5.2-Codex" | "GPT-5.2 Codex" => "gpt-5.2-codex",
        "GPT-5.2" => "gpt-5.2",
        "GPT-5.1-Codex" | "GPT-5.1 Codex" => "gpt-5.1-codex",
        "GPT-5.1" => "gpt-5.1",
        "Gemini 3.1 Pro" => "gemini-3.1-pro",
        "Gemini 3.1" => "gemini-3.1",
        "GLM 5.1" | "GLM-5.1" => "glm-5.1",
        "Claude Sonnet 4.6" | "Claude-Sonnet-4.6" => "claude-sonnet-4.6",
        "Claude Sonnet 4.5" | "Claude-Sonnet-4.5" => "claude-sonnet-4.5",
        other => other,
    }
    .to_string()
}

/// Infer the provider from the display name.
fn provider_for_model(name: &str) -> &'static str {
    if name.contains("GPT") || name.contains("gpt") {
        "openai"
    } else if name.contains("Claude") || name.contains("claude") {
        "anthropic"
    } else if name.contains("Gemini") || name.contains("gemini") {
        "google"
    } else if name.contains("GLM") || name.contains("glm") {
        "zhipu"
    } else {
        "trae"
    }
}

/// Parse a single session JSON object into a `UnifiedMessage`.
fn parse_session(client: &str, session: &serde_json::Value) -> Option<UnifiedMessage> {
    let model_raw = session["model_name"].as_str().unwrap_or("");
    let mode = session["mode"].as_str().unwrap_or("");
    // Auto-mode sessions come back with `model_name: ""` because the
    // system picks a model per turn. Bucket them under `trae-<mode>` (e.g.
    // `trae-auto`) so the cost is still attributed instead of disappearing
    // into an empty Model cell.
    let model_id = if !model_raw.is_empty() {
        normalize_trae_model(model_raw)
    } else if !mode.is_empty() {
        format!("trae-{}", mode.to_ascii_lowercase())
    } else {
        "trae-unknown".to_string()
    };
    let provider = provider_for_model(&model_id);
    // Records without a real `session_id` cannot be deduplicated correctly
    // (every "missing-id" record would collide on the same key); records
    // without a positive `usage_time` would land at epoch 0. Drop them
    // rather than fabricating placeholders.
    let session_id = session["session_id"].as_str()?;
    let usage_time = session["usage_time"].as_i64()?;
    if session_id.is_empty() || usage_time <= 0 {
        return None;
    }
    // API returns epoch seconds; UnifiedMessage expects milliseconds. Use
    // `checked_mul` because the JSON cache is untrusted input — a crafted
    // `usage_time` near `i64::MAX` would panic in debug builds and silently
    // wrap to a negative timestamp in release builds.
    let timestamp_ms = usage_time.checked_mul(1000)?;
    let cost = session["dollar_float"].as_f64().unwrap_or(0.0);

    let extra = &session["extra_info"];
    let input = extra["input_token"].as_i64().unwrap_or(0);
    let output = extra["output_token"].as_i64().unwrap_or(0);
    let cache_read = extra["cache_read_token"].as_i64().unwrap_or(0);
    let cache_write = extra["cache_write_token"].as_i64().unwrap_or(0);

    if input + output + cache_read + cache_write == 0 {
        return None;
    }

    let dedup_key = Some(format!("trae:{}:{}", session_id, usage_time));

    Some(UnifiedMessage::new_with_dedup(
        client,
        model_id,
        provider,
        session_id,
        timestamp_ms,
        TokenBreakdown {
            input,
            output,
            cache_read,
            cache_write,
            reasoning: 0,
        },
        cost,
        dedup_key,
    ))
}

/// Parse a cache file containing an array of sessions as returned by the API.
pub fn parse_trae_file(client: &str, path: &std::path::Path) -> Vec<UnifiedMessage> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    let value: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let sessions = match value.as_array() {
        Some(a) => a,
        None => return Vec::new(),
    };
    sessions
        .iter()
        .filter_map(|s| parse_session(client, s))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_fixture(data: &str) -> tempfile::NamedTempFile {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(data.as_bytes()).unwrap();
        f.flush().unwrap();
        f
    }

    #[test]
    fn test_parse_empty_file() {
        let f = write_fixture("[]");
        let msgs = parse_trae_file("trae", f.path());
        assert!(msgs.is_empty());
    }

    #[test]
    fn test_parse_single_session() {
        let json = serde_json::json!([{
            "model_name": "GPT-5.4",
            "session_id": "test-session-1",
            "usage_time": 1776000000,
            "dollar_float": 0.5,
            "extra_info": {
                "input_token": 1000,
                "output_token": 500,
                "cache_read_token": 200,
                "cache_write_token": 100
            }
        }]);
        let f = write_fixture(&json.to_string());
        let msgs = parse_trae_file("trae", f.path());
        assert_eq!(msgs.len(), 1);
        let m = &msgs[0];
        assert_eq!(m.client, "trae");
        assert_eq!(m.model_id, "gpt-5.4");
        assert_eq!(m.provider_id, "openai");
        assert_eq!(m.tokens.input, 1000);
        assert_eq!(m.tokens.output, 500);
        assert_eq!(m.tokens.cache_read, 200);
        assert_eq!(m.tokens.cache_write, 100);
        assert_eq!(m.cost, 0.5);
        // timestamp: epoch seconds → ms
        assert_eq!(m.timestamp, 1776000000_000);
    }

    #[test]
    fn test_skip_zero_token_session() {
        let json = serde_json::json!([{
            "model_name": "GPT-5.4",
            "session_id": "empty-session",
            "usage_time": 1776000000,
            "dollar_float": 0.0,
            "extra_info": {
                "input_token": 0,
                "output_token": 0,
                "cache_read_token": 0,
                "cache_write_token": 0
            }
        }]);
        let f = write_fixture(&json.to_string());
        let msgs = parse_trae_file("trae", f.path());
        assert!(msgs.is_empty());
    }

    #[test]
    fn test_normalize_model_names() {
        assert_eq!(normalize_trae_model("GPT-5.4"), "gpt-5.4");
        assert_eq!(normalize_trae_model("GPT-5.3-Codex"), "gpt-5.3-codex");
        assert_eq!(normalize_trae_model("GPT-5.3 Codex"), "gpt-5.3-codex");
        assert_eq!(normalize_trae_model("Gemini 3.1 Pro"), "gemini-3.1-pro");
        assert_eq!(normalize_trae_model("GLM 5.1"), "glm-5.1");
        assert_eq!(normalize_trae_model("Unknown Model"), "Unknown Model");
    }

    #[test]
    fn test_provider_mapping() {
        assert_eq!(provider_for_model("GPT-5.4"), "openai");
        assert_eq!(provider_for_model("Claude Sonnet 4.6"), "anthropic");
        assert_eq!(provider_for_model("Gemini 3.1 Pro"), "google");
        assert_eq!(provider_for_model("GLM 5.1"), "zhipu");
        assert_eq!(provider_for_model("SomeOtherModel"), "trae");
    }

    #[test]
    fn test_auto_mode_fallback_uses_mode_as_model() {
        // Trae's "Auto" mode returns `model_name: ""` because no single
        // model is bound to the session. The parser must still keep the
        // cost and bucket it under `trae-auto`.
        let json = serde_json::json!([{
            "model_name": "",
            "mode": "Auto",
            "session_id": "auto-session-1",
            "usage_time": 1776000000,
            "dollar_float": 0.27,
            "extra_info": {
                "input_token": 159213,
                "output_token": 210,
                "cache_read_token": 6144,
                "cache_write_token": 0
            }
        }]);
        let f = write_fixture(&json.to_string());
        let msgs = parse_trae_file("trae", f.path());
        assert_eq!(msgs.len(), 1);
        let m = &msgs[0];
        assert_eq!(m.model_id, "trae-auto");
        assert_eq!(m.provider_id, "trae");
        assert_eq!(m.cost, 0.27);
    }

    #[test]
    fn test_skip_session_without_session_id() {
        // A record without `session_id` would otherwise dedup to the same
        // key as every other malformed record. Drop it instead.
        let json = serde_json::json!([{
            "model_name": "GPT-5.4",
            "usage_time": 1776000000,
            "dollar_float": 0.1,
            "extra_info": { "input_token": 100, "output_token": 1, "cache_read_token": 0, "cache_write_token": 0 }
        }]);
        let f = write_fixture(&json.to_string());
        let msgs = parse_trae_file("trae", f.path());
        assert!(msgs.is_empty());
    }

    #[test]
    fn test_skip_session_without_usage_time() {
        // No `usage_time` → would land at epoch 0. Drop it.
        let json = serde_json::json!([{
            "model_name": "GPT-5.4",
            "session_id": "abc",
            "dollar_float": 0.1,
            "extra_info": { "input_token": 100, "output_token": 1, "cache_read_token": 0, "cache_write_token": 0 }
        }]);
        let f = write_fixture(&json.to_string());
        let msgs = parse_trae_file("trae", f.path());
        assert!(msgs.is_empty());
    }

    #[test]
    fn test_skip_session_with_non_positive_usage_time() {
        let json = serde_json::json!([{
            "model_name": "GPT-5.4",
            "session_id": "abc",
            "usage_time": 0,
            "dollar_float": 0.1,
            "extra_info": { "input_token": 100, "output_token": 1, "cache_read_token": 0, "cache_write_token": 0 }
        }]);
        let f = write_fixture(&json.to_string());
        let msgs = parse_trae_file("trae", f.path());
        assert!(msgs.is_empty());
    }

    #[test]
    fn test_skip_session_with_overflowing_usage_time() {
        // A maliciously crafted cache could contain a near-MAX `usage_time`.
        // Multiplying by 1000 would overflow `i64` — debug-panic or wrap to
        // a negative timestamp. Reject the record instead.
        let json = serde_json::json!([{
            "model_name": "GPT-5.4",
            "session_id": "evil",
            "usage_time": i64::MAX,
            "dollar_float": 0.1,
            "extra_info": { "input_token": 100, "output_token": 1, "cache_read_token": 0, "cache_write_token": 0 }
        }]);
        let f = write_fixture(&json.to_string());
        let msgs = parse_trae_file("trae", f.path());
        assert!(msgs.is_empty());
    }

    #[test]
    fn test_missing_model_and_mode_falls_back_to_unknown() {
        let json = serde_json::json!([{
            "session_id": "no-meta",
            "usage_time": 1776000000,
            "dollar_float": 0.01,
            "extra_info": { "input_token": 100, "output_token": 1, "cache_read_token": 0, "cache_write_token": 0 }
        }]);
        let f = write_fixture(&json.to_string());
        let msgs = parse_trae_file("trae", f.path());
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].model_id, "trae-unknown");
    }
}
