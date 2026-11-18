//! Syok Code usage history parser.
//!
//! Syok Code (<https://syok-ai.com/syok-code>) is a terminal coding agent that
//! talks to an OpenAI-compatible router which can switch providers between
//! steps. It appends one JSON line per model call to a monthly file,
//! `~/.syok/usage/YYYY-MM.jsonl`:
//!
//! ```json
//! {"v":1,"ts":"2026-11-18T19:41:02.511Z","session":"20261118194101-ab12","project":"/work/shop",
//!  "client":"syok-code","requested":"syok-coder","provider":"claude","model":"claude-sonnet-5",
//!  "input":5210,"output":312,"cache_read":4096,"reasoning":120,"total":5522}
//! ```
//!
//! The counts come straight from the router's OpenAI-style `usage` block, so
//! `input` (prompt tokens) already contains `cache_read`, and `output`
//! (completion tokens) already contains `reasoning`. Both are split apart here,
//! because `TokenBreakdown` buckets are additive.
//!
//! `provider` and `model` name who really answered (the router can fall over
//! to another provider mid-session); `requested` is the model or preset the
//! user asked for and is only a fallback model id. The file never holds message
//! text. It is append-only, so a line's position in its month file is a stable
//! identity for deduplication.

use super::utils::{for_each_json_line, parse_timestamp_str};
use super::{normalize_workspace_key, workspace_label_from_key, UnifiedMessage};
use crate::provider_identity::inferred_provider_from_model;
use crate::TokenBreakdown;
use serde::Deserialize;
use std::path::Path;

pub const CLIENT_ID: &str = "syok-code";

#[derive(Debug, Deserialize)]
struct SyokUsageLine {
    v: Option<i64>,
    ts: Option<String>,
    session: Option<String>,
    project: Option<String>,
    requested: Option<String>,
    provider: Option<String>,
    model: Option<String>,
    input: Option<i64>,
    output: Option<i64>,
    cache_read: Option<i64>,
    reasoning: Option<i64>,
}

fn non_empty(value: &Option<String>) -> Option<&str> {
    value.as_deref().map(str::trim).filter(|s| !s.is_empty())
}

/// Map Syok's provider names to the canonical provider ids used for pricing,
/// for when the model id alone does not name its vendor.
fn canonical_provider(provider: &str) -> Option<&'static str> {
    match provider.to_ascii_lowercase().as_str() {
        "claude" | "anthropic" => Some("anthropic"),
        "openai" | "chatgpt" => Some("openai"),
        "gemini" | "google" => Some("google"),
        "grok" | "xai" => Some("xai"),
        "mistral" => Some("mistral"),
        "deepseek" => Some("deepseek"),
        "openrouter" => Some("openrouter"),
        "groq" => Some("groq"),
        _ => None,
    }
}

pub fn parse_syok_code_file(path: &Path) -> Vec<UnifiedMessage> {
    let file_key = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut messages = Vec::new();

    for_each_json_line(path, &mut |index, line| {
        let Ok(row) = serde_json::from_str::<SyokUsageLine>(line) else {
            return;
        };
        // Only the format version this parser knows. A future version is
        // skipped rather than misread.
        if row.v != Some(1) {
            return;
        }
        let Some(timestamp) = row.ts.as_deref().and_then(parse_timestamp_str) else {
            return;
        };

        let input = row.input.unwrap_or(0).max(0);
        let output = row.output.unwrap_or(0).max(0);
        let cache_read = row.cache_read.unwrap_or(0).max(0).min(input);
        let reasoning = row.reasoning.unwrap_or(0).max(0).min(output);
        let tokens = TokenBreakdown {
            input: input - cache_read,
            output: output - reasoning,
            cache_read,
            cache_write: 0,
            cache_write_1h: 0,
            reasoning,
        };
        if tokens.total() == 0 {
            return;
        }

        let model = non_empty(&row.model)
            .or_else(|| non_empty(&row.requested))
            .unwrap_or("unknown");
        let provider = inferred_provider_from_model(model)
            .or_else(|| non_empty(&row.provider).and_then(canonical_provider))
            .map(str::to_string)
            .or_else(|| non_empty(&row.provider).map(str::to_string))
            .unwrap_or_else(|| CLIENT_ID.to_string());
        let session = non_empty(&row.session).unwrap_or("unknown");

        let mut message =
            UnifiedMessage::new(CLIENT_ID, model, provider, session, timestamp, tokens, 0.0);
        message.dedup_key = Some(format!("{CLIENT_ID}:{file_key}:{index}"));
        let workspace_key = non_empty(&row.project).and_then(normalize_workspace_key);
        let workspace_label = workspace_key.as_deref().and_then(workspace_label_from_key);
        message.set_workspace(workspace_key, workspace_label);
        messages.push(message);
    });

    messages
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn file_with(lines: &[&str]) -> NamedTempFile {
        let mut file = NamedTempFile::new().unwrap();
        for line in lines {
            writeln!(file, "{line}").unwrap();
        }
        file.flush().unwrap();
        file
    }

    #[test]
    fn test_parse_syok_code_line_splits_cache_and_reasoning() {
        let file = file_with(&[
            r#"{"v":1,"ts":"2026-11-18T19:41:02.511Z","session":"20261118194101-ab12","project":"/work/shop","client":"syok-code","requested":"syok-coder","provider":"claude","model":"claude-sonnet-5","input":5210,"output":312,"cache_read":4096,"reasoning":120,"total":5522}"#,
        ]);

        let messages = parse_syok_code_file(file.path());

        assert_eq!(messages.len(), 1);
        let m = &messages[0];
        assert_eq!(m.client, "syok-code");
        assert_eq!(m.session_id, "20261118194101-ab12");
        assert_eq!(m.model_id, "claude-sonnet-5");
        assert_eq!(m.provider_id, "anthropic");
        assert_eq!(m.tokens.input, 5210 - 4096);
        assert_eq!(m.tokens.cache_read, 4096);
        assert_eq!(m.tokens.output, 312 - 120);
        assert_eq!(m.tokens.reasoning, 120);
        // the split keeps the router's own total
        assert_eq!(m.tokens.total(), 5210 + 312);
        assert_eq!(
            m.timestamp,
            chrono::DateTime::parse_from_rfc3339("2026-11-18T19:41:02.511Z")
                .unwrap()
                .timestamp_millis()
        );
        assert_eq!(m.workspace_key.as_deref(), Some("/work/shop"));
        assert_eq!(m.workspace_label.as_deref(), Some("shop"));
        assert!(m.dedup_key.as_deref().unwrap().starts_with("syok-code:"));
    }

    #[test]
    fn test_parse_syok_code_provider_falls_back_from_model_to_router_name() {
        let file = file_with(&[
            // model names its vendor: the model wins
            r#"{"v":1,"ts":"2026-11-18T01:00:00.000Z","session":"s","provider":"mistral","model":"gpt-5","input":10,"output":1}"#,
            // model does not: Syok's provider name is mapped to the canonical id
            r#"{"v":1,"ts":"2026-11-18T01:00:01.000Z","session":"s","provider":"grok","model":"some-new-model","input":10,"output":1}"#,
            // nobody said who answered: the requested preset is the model, the client the provider
            r#"{"v":1,"ts":"2026-11-18T01:00:02.000Z","session":"s","provider":"","model":"","requested":"syok-fast","input":10,"output":1}"#,
        ]);

        let messages = parse_syok_code_file(file.path());

        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].provider_id, "openai");
        assert_eq!(messages[1].provider_id, "xai");
        assert_eq!(messages[2].model_id, "syok-fast");
        assert_eq!(messages[2].provider_id, "syok-code");
    }

    #[test]
    fn test_parse_syok_code_skips_bad_lines_and_clamps() {
        let file = file_with(&[
            "{not json",
            "",
            r#"{"v":2,"ts":"2026-11-18T01:00:00.000Z","input":10,"output":1}"#,
            r#"{"v":1,"input":10,"output":1}"#,
            r#"{"v":1,"ts":"2026-11-18T01:00:00.000Z","input":0,"output":0}"#,
            r#"{"v":1,"ts":"2026-11-18T01:00:00.000Z","session":"s","model":"claude-sonnet-5","input":-5,"output":7,"cache_read":99,"reasoning":50}"#,
        ]);

        let messages = parse_syok_code_file(file.path());

        assert_eq!(messages.len(), 1);
        let t = &messages[0].tokens;
        assert_eq!((t.input, t.cache_read, t.output, t.reasoning), (0, 0, 0, 7));
        // a blank line does not shift the identity of the lines after it
        assert_eq!(
            messages[0].dedup_key.as_deref().unwrap().rsplit(':').next(),
            Some("5")
        );
    }
}
