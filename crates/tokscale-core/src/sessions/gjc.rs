//! gajae-code (`gjc`) session parser
//!
//! Parses JSONL session files from `~/.gjc/agent/sessions/<project-slug>/*.jsonl`
//! (and depth-2 per-pass sub-agent children `<slug>/<session>/N-*.jsonl`).
//!
//! Each line is tagged by `type`:
//! - `session` — header carrying `id` (session id) and `cwd` (workspace). No
//!   message is emitted for it.
//! - `service_tier_change` — skipped.
//! - `message` — emits ONLY assistant messages. The assistant `message` object
//!   carries `model`/`provider`/`api`, a unix-ms `timestamp`, and a `usage`
//!   object that includes an authoritative `usage.cost` (USD) breakdown.
//!
//! Cost policy (A1): the embedded `usage.cost.total` (USD) is reused verbatim
//! when present, finite, and non-negative. Otherwise cost is left at `0.0` so
//! the lib.rs dispatch Hermes guard can reprice from tokens.
//!
//! Dedup (codebuff-style): a stable `dedup_key` of `<session id>:<message id>`
//! is preferred; when ids are absent a deterministic fallback derived from the
//! session, timestamp, model and token breakdown keeps structurally identical
//! replays (depth-1 vs depth-2 files) collapsed to one message.

use super::utils::file_modified_timestamp_ms;
use super::{normalize_workspace_key, workspace_label_from_key, CostSource, UnifiedMessage};
use crate::provider_identity::inferred_provider_from_model;
use crate::TokenBreakdown;
use serde::Deserialize;
use std::io::{BufRead, BufReader};
use std::path::Path;

/// A single JSONL entry. The `session` header reuses `id`/`timestamp`/`cwd`;
/// `message` entries carry the assistant payload under `message`.
#[derive(Debug, Deserialize)]
struct GjcEntry {
    #[serde(rename = "type")]
    entry_type: String,
    id: Option<String>,
    /// Entry-level ISO-8601 timestamp (session header and message fallback).
    timestamp: Option<String>,
    /// Session header working directory.
    cwd: Option<String>,
    message: Option<GjcMessage>,
}

#[derive(Debug, Deserialize)]
struct GjcMessage {
    role: Option<String>,
    model: Option<String>,
    provider: Option<String>,
    #[allow(dead_code)]
    api: Option<String>,
    /// Unix-ms timestamp (preferred for ordering/date).
    timestamp: Option<i64>,
    usage: Option<GjcUsage>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GjcUsage {
    input: Option<i64>,
    output: Option<i64>,
    cache_read: Option<i64>,
    cache_write: Option<i64>,
    #[allow(dead_code)]
    total_tokens: Option<i64>,
    cost: Option<GjcCost>,
}

#[derive(Debug, Deserialize)]
struct GjcCost {
    /// Authoritative total cost in USD.
    total: Option<f64>,
}

/// Reuse the embedded `usage.cost.total` (USD) only when present, finite, and
/// non-negative. Otherwise return `0.0` so the dispatch pricing guard reprices.
fn embedded_cost(usage: &GjcUsage) -> (f64, CostSource) {
    match usage.cost.as_ref().and_then(|c| c.total) {
        Some(total) if total.is_finite() && total >= 0.0 => (total, CostSource::ProviderReported),
        _ => (0.0, CostSource::Unknown),
    }
}

/// Build a deterministic fallback dedup key for messages lacking a stable
/// upstream id, combining session, timestamp, model and token breakdown so
/// structurally identical replays collapse while distinct messages stay apart.
fn derive_dedup_key(
    session_id: &str,
    ts: i64,
    model: &str,
    provider: &str,
    tokens: &TokenBreakdown,
    line: &str,
) -> String {
    use sha2::{Digest, Sha256};

    let line_hash = Sha256::digest(line.as_bytes());

    format!(
        "gjc:{session_id}:{ts}:{model}:{provider}:{i}-{o}-{cr}-{cw}-{r}:{line_hash:x}",
        i = tokens.input,
        o = tokens.output,
        cr = tokens.cache_read,
        cw = tokens.cache_write,
        r = tokens.reasoning,
    )
}

/// Parse a gajae-code JSONL session file into UnifiedMessages.
///
/// Per-line parse: malformed/partial/legacy lines are skipped, never aborting
/// the file. The `session` header and `service_tier_change` lines emit nothing.
pub fn parse_gjc_file(path: &Path) -> Vec<UnifiedMessage> {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };

    let fallback_timestamp = file_modified_timestamp_ms(path);

    let reader = BufReader::new(file);
    let mut messages: Vec<UnifiedMessage> = Vec::with_capacity(64);
    let mut buffer = Vec::with_capacity(4096);

    let mut session_id: Option<String> = None;
    let mut workspace_key: Option<String> = None;
    let mut workspace_label: Option<String> = None;

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        buffer.clear();
        buffer.extend_from_slice(trimmed.as_bytes());
        let entry = match simd_json::from_slice::<GjcEntry>(&mut buffer) {
            Ok(e) => e,
            Err(_) => continue,
        };

        match entry.entry_type.as_str() {
            "session" => {
                if let Some(id) = entry.id {
                    session_id = Some(id);
                }
                if let Some(key) = entry.cwd.as_deref().and_then(normalize_workspace_key) {
                    workspace_label = workspace_label_from_key(&key);
                    workspace_key = Some(key);
                }
                continue;
            }
            "message" => {}
            // service_tier_change and any other entry types: skip.
            _ => continue,
        }

        let message = match entry.message {
            Some(m) => m,
            None => continue,
        };

        if message.role.as_deref() != Some("assistant") {
            continue;
        }

        let usage = match message.usage {
            Some(u) => u,
            None => continue,
        };

        let model = match message.model {
            Some(m) => m,
            None => continue,
        };

        // A missing provider field is recoverable: infer it from the model name
        // (and fall back to "gjc") rather than dropping a message that carries
        // valid tokens.
        let provider = match message.provider {
            Some(p) => p,
            None => inferred_provider_from_model(&model)
                .unwrap_or("gjc")
                .to_string(),
        };

        // Prefer unix-ms message timestamp; fall back to entry ISO timestamp,
        // then the file mtime.
        let timestamp = message.timestamp.unwrap_or_else(|| {
            entry
                .timestamp
                .and_then(|ts| chrono::DateTime::parse_from_rfc3339(&ts).ok())
                .map(|dt| dt.timestamp_millis())
                .unwrap_or(fallback_timestamp)
        });

        let tokens = TokenBreakdown {
            input: usage.input.unwrap_or(0).max(0),
            output: usage.output.unwrap_or(0).max(0),
            cache_read: usage.cache_read.unwrap_or(0).max(0),
            cache_write: usage.cache_write.unwrap_or(0).max(0),
            reasoning: 0,
        };

        let (cost, cost_source) = embedded_cost(&usage);

        // No `{"type":"session",...}` header in this file: fall back to the file
        // name rather than a shared `"unknown"`, so two independent header-less
        // files do not collide on the same session in the cross-file dedup set.
        //
        // Caveat: a header-less depth-2 replay keys off its own (per-pass) file
        // stem, so it will NOT collapse against a header-less depth-1 parent the
        // way a shared session id would. The documented depth-1/depth-2
        // replay-collapse guarantee (see module doc above and lib.rs dispatch)
        // therefore holds for HEADERED files only — the realistic case, since
        // real gjc sessions always carry a `{"type":"session"}` header.
        let session = session_id.clone().unwrap_or_else(|| {
            path.file_stem()
                .and_then(|stem| stem.to_str())
                .map(str::to_string)
                .unwrap_or_else(|| "unknown".to_string())
        });
        let dedup_key = match entry.id.filter(|s| !s.is_empty()) {
            Some(msg_id) => format!("{session}:{msg_id}"),
            None => derive_dedup_key(&session, timestamp, &model, &provider, &tokens, trimmed),
        };

        let mut unified = UnifiedMessage::new_with_dedup(
            "gjc",
            model,
            provider,
            session,
            timestamp,
            tokens,
            cost,
            Some(dedup_key),
        );
        if cost_source == CostSource::ProviderReported {
            unified.mark_provider_reported_cost();
        }
        unified.set_workspace(workspace_key.clone(), workspace_label.clone());
        messages.push(unified);
    }

    messages
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn create_test_file(content: &str) -> NamedTempFile {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(content.as_bytes()).unwrap();
        file.flush().unwrap();
        file
    }

    #[test]
    fn test_parse_gjc_assistant_line() {
        let content = r#"{"type":"session","id":"gjc_ses_001","timestamp":"2026-01-01T00:00:00.000Z","cwd":"/work/pi"}
{"type":"message","id":"msg_001","parentId":null,"timestamp":"2026-01-01T00:00:01.000Z","message":{"role":"assistant","model":"claude-sonnet-4","provider":"anthropic","api":"anthropic","timestamp":1767225601000,"usage":{"input":100,"output":50,"cacheRead":10,"cacheWrite":5,"totalTokens":165,"cost":{"input":0.1,"output":0.2,"cacheRead":0.0,"cacheWrite":0.0,"total":0.3}}}}"#;
        let file = create_test_file(content);
        let messages = parse_gjc_file(file.path());

        assert_eq!(messages.len(), 1);
        let m = &messages[0];
        assert_eq!(m.client, "gjc");
        assert_eq!(m.session_id, "gjc_ses_001");
        assert_eq!(m.model_id, "claude-sonnet-4");
        assert_eq!(m.provider_id, "anthropic");
        assert_eq!(m.tokens.input, 100);
        assert_eq!(m.tokens.output, 50);
        assert_eq!(m.tokens.cache_read, 10);
        assert_eq!(m.tokens.cache_write, 5);
        assert_eq!(m.tokens.reasoning, 0);
        assert_eq!(m.timestamp, 1767225601000);
        assert_eq!(m.workspace_key, Some("/work/pi".to_string()));
        assert_eq!(m.workspace_label, Some("pi".to_string()));
    }

    #[test]
    fn test_parse_gjc_skips_header_and_service_tier_change() {
        let content = r#"{"type":"session","id":"gjc_ses_002","timestamp":"2026-01-01T00:00:00.000Z","cwd":"/tmp"}
{"type":"service_tier_change","id":"x","timestamp":"2026-01-01T00:00:00.500Z"}
{"type":"message","id":"msg_002","timestamp":"2026-01-01T00:00:01.000Z","message":{"role":"assistant","model":"gpt-4o","provider":"openai","timestamp":1767225601000,"usage":{"input":1,"output":1,"cost":{"total":0.01}}}}"#;
        let file = create_test_file(content);
        let messages = parse_gjc_file(file.path());
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "gpt-4o");
    }

    #[test]
    fn test_parse_gjc_skips_malformed_line() {
        let content = r#"{"type":"session","id":"gjc_ses_003","cwd":"/tmp"}
not valid json at all
{"type":"message","id":"msg_003","message":{"role":"assistant","model":"gpt-4o-mini","provider":"openai","timestamp":1767225601000,"usage":{"input":10,"output":5,"cost":{"total":0.02}}}}"#;
        let file = create_test_file(content);
        let messages = parse_gjc_file(file.path());
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "gpt-4o-mini");
    }

    #[test]
    fn test_parse_gjc_headerless_files_get_distinct_sessions_from_filename() {
        // Two independent files with no `{"type":"session"}` header, each with
        // the same message id. The session falls back to the file name, so they
        // get distinct sessions and dedup keys instead of colliding on
        // "unknown:msg_1".
        let dir = tempfile::tempdir().unwrap();
        let line = r#"{"type":"message","id":"msg_1","message":{"role":"assistant","model":"gpt-4o","provider":"openai","timestamp":1767225601000,"usage":{"input":1,"output":1,"cost":{"total":0.01}}}}"#;
        let path_a = dir.path().join("session_a.jsonl");
        let path_b = dir.path().join("session_b.jsonl");
        std::fs::write(&path_a, line).unwrap();
        std::fs::write(&path_b, line).unwrap();

        let a = parse_gjc_file(&path_a);
        let b = parse_gjc_file(&path_b);
        assert_eq!(a.len(), 1);
        assert_eq!(b.len(), 1);
        assert_eq!(a[0].session_id, "session_a");
        assert_eq!(b[0].session_id, "session_b");
        assert_ne!(a[0].dedup_key, b[0].dedup_key);
    }

    #[test]
    fn test_parse_gjc_header_session_id_wins_over_file_stem() {
        // The file HAS a `{"type":"session"}` header whose id deliberately
        // differs from the file stem. The file-stem fallback must apply only
        // when no header is present, so the session id is taken from the header
        // and the (colliding-looking) stem is ignored.
        let dir = tempfile::tempdir().unwrap();
        let content = r#"{"type":"session","id":"gjc_ses_header","cwd":"/tmp"}
{"type":"message","id":"msg_1","message":{"role":"assistant","model":"gpt-4o","provider":"openai","timestamp":1767225601000,"usage":{"input":1,"output":1,"cost":{"total":0.01}}}}"#;
        // Stem is "unknown" on purpose: if the fallback ever leaked past a
        // present header, the session id would read "unknown" and this fails.
        let path = dir.path().join("unknown.jsonl");
        std::fs::write(&path, content).unwrap();

        let messages = parse_gjc_file(&path);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].session_id, "gjc_ses_header");
        assert_eq!(
            messages[0].dedup_key,
            Some("gjc_ses_header:msg_1".to_string())
        );
    }

    #[test]
    fn test_parse_gjc_skips_non_assistant() {
        let content = r#"{"type":"session","id":"gjc_ses_004","cwd":"/tmp"}
{"type":"message","id":"msg_u","message":{"role":"user","model":"gpt-4o","provider":"openai","timestamp":1767225601000,"usage":{"input":10,"output":5,"cost":{"total":0.02}}}}"#;
        let file = create_test_file(content);
        assert!(parse_gjc_file(file.path()).is_empty());
    }

    #[test]
    fn test_parse_gjc_sets_dedup_key() {
        let content = r#"{"type":"session","id":"gjc_ses_005","cwd":"/tmp"}
{"type":"message","id":"msg_abc","message":{"role":"assistant","model":"gpt-4o","provider":"openai","timestamp":1767225601000,"usage":{"input":10,"output":5,"cost":{"total":0.02}}}}"#;
        let file = create_test_file(content);
        let messages = parse_gjc_file(file.path());
        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0].dedup_key,
            Some("gjc_ses_005:msg_abc".to_string())
        );
    }

    #[test]
    fn test_parse_gjc_dedup_key_fallback_when_id_absent() {
        let content = r#"{"type":"session","id":"gjc_ses_006","cwd":"/tmp"}
{"type":"message","message":{"role":"assistant","model":"gpt-4o","provider":"openai","timestamp":1767225601000,"usage":{"input":10,"output":5,"cost":{"total":0.02}}}}"#;
        let file = create_test_file(content);
        let messages = parse_gjc_file(file.path());
        assert_eq!(messages.len(), 1);
        let key = messages[0].dedup_key.clone().unwrap();
        assert!(
            key.starts_with("gjc:gjc_ses_006:1767225601000:gpt-4o:openai:10-5-0-0-0:"),
            "key={key}"
        );
    }

    #[test]
    fn test_parse_gjc_reads_embedded_cost_total() {
        let content = r#"{"type":"session","id":"gjc_ses_007","cwd":"/tmp"}
{"type":"message","id":"msg_c","message":{"role":"assistant","model":"some-model","provider":"anthropic","timestamp":1767225601000,"usage":{"input":10,"output":5,"cost":{"input":0.5,"output":0.7,"total":1.25}}}}"#;
        let file = create_test_file(content);
        let messages = parse_gjc_file(file.path());
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].cost, 1.25);
    }

    #[test]
    fn test_parse_gjc_cost_zero_when_absent_or_invalid() {
        let content = r#"{"type":"session","id":"gjc_ses_008","cwd":"/tmp"}
{"type":"message","id":"msg_nocost","message":{"role":"assistant","model":"some-model","provider":"anthropic","timestamp":1767225601000,"usage":{"input":10,"output":5}}}"#;
        let file = create_test_file(content);
        let messages = parse_gjc_file(file.path());
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].cost, 0.0);
    }
    // ── Adversarial / red-team tests ────────────────────────────────────────

    /// (a) Completely empty file -> empty vec
    #[test]
    fn test_adv_empty_file_returns_empty_vec() {
        let file = create_test_file("");
        let messages = parse_gjc_file(file.path());
        assert!(messages.is_empty(), "expected empty vec for empty file");
    }

    /// (b) File with only a session header -> empty vec (no message lines)
    #[test]
    fn test_adv_only_session_header_returns_empty_vec() {
        let content = r#"{"type":"session","id":"gjc_adv_b","cwd":"/work/myproject"}"#;
        let file = create_test_file(content);
        let messages = parse_gjc_file(file.path());
        assert!(
            messages.is_empty(),
            "expected empty vec when only header present"
        );
    }

    /// (c) Malformed JSON line sandwiched between two valid messages -> only
    ///     the two valid ones are parsed; the bad line is silently skipped.
    #[test]
    fn test_adv_malformed_line_between_valid_messages_skipped() {
        let content = r#"{"type":"session","id":"gjc_adv_c","cwd":"/tmp"}
{"type":"message","id":"msg_c1","message":{"role":"assistant","model":"model-a","provider":"prov-a","timestamp":1700000001000,"usage":{"input":10,"output":5,"cost":{"total":0.01}}}}
{this is not valid json !!
{"type":"message","id":"msg_c2","message":{"role":"assistant","model":"model-b","provider":"prov-b","timestamp":1700000002000,"usage":{"input":20,"output":10,"cost":{"total":0.02}}}}"#;
        let file = create_test_file(content);
        let messages = parse_gjc_file(file.path());
        assert_eq!(messages.len(), 2, "expected exactly 2 valid messages");
        assert_eq!(messages[0].model_id, "model-a");
        assert_eq!(messages[1].model_id, "model-b");
    }

    /// (d) Message missing model -> skipped, no panic.
    ///     Message missing provider -> KEPT with an inferred/fallback provider
    ///       (a missing provider is recoverable, not a drop condition).
    ///     Message missing usage -> skipped, no panic.
    #[test]
    fn test_adv_missing_model_provider_usage_skipped_no_panic() {
        // missing model
        let content_no_model = r#"{"type":"session","id":"gjc_adv_d1","cwd":"/tmp"}
{"type":"message","id":"no_model","message":{"role":"assistant","provider":"prov","timestamp":1700000001000,"usage":{"input":1,"output":1,"cost":{"total":0.001}}}}"#;
        let file = create_test_file(content_no_model);
        assert!(
            parse_gjc_file(file.path()).is_empty(),
            "missing model should be skipped"
        );

        // missing provider: recoverable. The model "m" yields no inferred
        // provider, so the parser falls back to "gjc" and keeps the message
        // rather than dropping its valid tokens.
        let content_no_provider = r#"{"type":"session","id":"gjc_adv_d2","cwd":"/tmp"}
{"type":"message","id":"no_prov","message":{"role":"assistant","model":"m","timestamp":1700000001000,"usage":{"input":1,"output":1,"cost":{"total":0.001}}}}"#;
        let file = create_test_file(content_no_provider);
        let no_provider_messages = parse_gjc_file(file.path());
        assert_eq!(
            no_provider_messages.len(),
            1,
            "missing provider should be recovered, not dropped"
        );
        assert_eq!(no_provider_messages[0].provider_id, "gjc");

        // missing usage
        let content_no_usage = r#"{"type":"session","id":"gjc_adv_d3","cwd":"/tmp"}
{"type":"message","id":"no_usage","message":{"role":"assistant","model":"m","provider":"p","timestamp":1700000001000}}"#;
        let file = create_test_file(content_no_usage);
        assert!(
            parse_gjc_file(file.path()).is_empty(),
            "missing usage should be skipped"
        );
    }

    /// (e) Negative token values are clamped to >= 0.
    #[test]
    fn test_adv_negative_token_values_clamped_to_zero() {
        let content = r#"{"type":"session","id":"gjc_adv_e","cwd":"/tmp"}
{"type":"message","id":"msg_neg","message":{"role":"assistant","model":"m","provider":"p","timestamp":1700000001000,"usage":{"input":-100,"output":-50,"cacheRead":-10,"cacheWrite":-5,"cost":{"total":0.0}}}}"#;
        let file = create_test_file(content);
        let messages = parse_gjc_file(file.path());
        assert_eq!(
            messages.len(),
            1,
            "message should be parsed despite negative tokens"
        );
        let t = &messages[0].tokens;
        assert_eq!(t.input, 0, "negative input clamped to 0");
        assert_eq!(t.output, 0, "negative output clamped to 0");
        assert_eq!(t.cache_read, 0, "negative cache_read clamped to 0");
        assert_eq!(t.cache_write, 0, "negative cache_write clamped to 0");
    }

    /// (f) Embedded cost.total negative -> falls back to 0.0.
    #[test]
    fn test_adv_negative_cost_total_falls_back_to_zero() {
        let content = r#"{"type":"session","id":"gjc_adv_f","cwd":"/tmp"}
{"type":"message","id":"msg_negcost","message":{"role":"assistant","model":"m","provider":"p","timestamp":1700000001000,"usage":{"input":5,"output":3,"cost":{"total":-9.99}}}}"#;
        let file = create_test_file(content);
        let messages = parse_gjc_file(file.path());
        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0].cost, 0.0,
            "negative cost.total must fall back to 0.0"
        );
    }

    /// (g) cost.total absent entirely -> 0.0.
    #[test]
    fn test_adv_absent_cost_total_is_zero() {
        let content = r#"{"type":"session","id":"gjc_adv_g","cwd":"/tmp"}
{"type":"message","id":"msg_nocosttotal","message":{"role":"assistant","model":"m","provider":"p","timestamp":1700000001000,"usage":{"input":5,"output":3,"cost":{"input":0.01}}}}"#;
        let file = create_test_file(content);
        let messages = parse_gjc_file(file.path());
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].cost, 0.0, "absent cost.total must be 0.0");
    }

    /// (h) Two messages with identical id + session -> identical dedup_key
    ///     (replay collapse: inserting both would produce the same key so a
    ///     dedup layer can collapse them).
    #[test]
    fn test_adv_same_id_and_session_produce_identical_dedup_key() {
        let line = r#"{"type":"message","id":"replay_msg","message":{"role":"assistant","model":"m","provider":"p","timestamp":1700000001000,"usage":{"input":10,"output":5,"cost":{"total":0.05}}}}"#;
        let content = format!(
            "{}\n{}\n{}",
            r#"{"type":"session","id":"gjc_ses_replay","cwd":"/tmp"}"#, line, line
        );
        let file = create_test_file(&content);
        let messages = parse_gjc_file(file.path());
        // Both lines parse successfully; it's the caller's job to dedup.
        assert_eq!(
            messages.len(),
            2,
            "parser emits both; dedup is caller's concern"
        );
        assert_eq!(
            messages[0].dedup_key, messages[1].dedup_key,
            "identical id+session must produce the same dedup_key"
        );
        assert_eq!(
            messages[0].dedup_key,
            Some("gjc_ses_replay:replay_msg".to_string())
        );
    }

    /// (i) Unicode / percent-encoded cwd in the session header normalizes
    ///     without panicking, and workspace_key/label are populated.
    #[test]
    fn test_adv_unicode_encoded_cwd_normalizes_without_panic() {
        // Path with non-ASCII Unicode characters
        let content_unicode = r#"{"type":"session","id":"gjc_adv_i1","cwd":"/home/用户/projects/my-app"}
{"type":"message","id":"msg_u","message":{"role":"assistant","model":"m","provider":"p","timestamp":1700000001000,"usage":{"input":1,"output":1,"cost":{"total":0.001}}}}"#;
        let file = create_test_file(content_unicode);
        // Must not panic; workspace fields may or may not be populated depending
        // on normalize_workspace_key, but the parse result must be exactly 1 message.
        let messages = parse_gjc_file(file.path());
        assert_eq!(
            messages.len(),
            1,
            "unicode cwd must not cause a panic or skip"
        );

        // Path with percent-encoding (URL-style directories some tools emit)
        let content_pct = r#"{"type":"session","id":"gjc_adv_i2","cwd":"/home/user/my%20project"}
{"type":"message","id":"msg_p","message":{"role":"assistant","model":"m","provider":"p","timestamp":1700000001000,"usage":{"input":1,"output":1,"cost":{"total":0.001}}}}"#;
        let file2 = create_test_file(content_pct);
        let messages2 = parse_gjc_file(file2.path());
        assert_eq!(
            messages2.len(),
            1,
            "percent-encoded cwd must not cause a panic or skip"
        );
    }
}
