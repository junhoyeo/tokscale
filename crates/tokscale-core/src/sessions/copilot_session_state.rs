//! GitHub Copilot CLI native session-state parser.
//!
//! Parses `~/.copilot/session-state/<session-id>/events.jsonl` files emitted by
//! GitHub Copilot CLI / agent. Each LLM turn emits a granular `session.usage_record`
//! event. When an older session log lacks per-request usage records, cumulative
//! `session.shutdown` deltas serve as a fallback.

use super::copilot_desktop::{collect_shutdown_usage, shutdown_deltas, ShutdownUsage};
use super::utils::{lossy_lines, parse_timestamp_str};
use super::{normalize_workspace_key, workspace_label_from_key, SharedStr, UnifiedMessage};
use crate::provider_identity::inferred_provider_from_model;
use crate::TokenBreakdown;
use serde_json::Value;
use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

const NANO_AIU_PER_USD: f64 = 1e11;

/// File-recorded turns take precedence over copies in the session-store DB.
/// Freeze the file timestamps before filtering DB rows so distinct store rows
/// never suppress each other. Sorting once avoids a full per-session scan for
/// every store row, including when event files are not in timestamp order.
pub(crate) struct SessionStateTurnIndex {
    timestamps: HashMap<SharedStr, Vec<i64>>,
}

impl SessionStateTurnIndex {
    pub(crate) fn new(mut timestamps: HashMap<SharedStr, Vec<i64>>) -> Self {
        for values in timestamps.values_mut() {
            values.sort_unstable();
            values.dedup();
        }
        Self { timestamps }
    }

    pub(crate) fn overlaps(&self, session_id: &str, timestamp: i64) -> bool {
        const MAX_TIMESTAMP_JITTER_MS: u64 = 20;
        let Some(timestamps) = self.timestamps.get(session_id) else {
            return false;
        };
        let lower_bound = timestamp.saturating_sub(MAX_TIMESTAMP_JITTER_MS as i64);
        let candidate = timestamps.partition_point(|value| *value < lower_bound);
        timestamps
            .get(candidate)
            .is_some_and(|value| value.abs_diff(timestamp) <= MAX_TIMESTAMP_JITTER_MS)
    }
}

pub fn parse_copilot_session_state_file(path: &Path) -> Vec<UnifiedMessage> {
    let mut messages = Vec::new();
    parse_copilot_session_state_file_into(path, &mut |_, msg| messages.push(msg));
    messages
}

pub(crate) fn parse_copilot_session_state_file_into(
    path: &Path,
    sink: &mut dyn FnMut(&Path, UnifiedMessage),
) {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(_) => return,
    };

    let fallback_timestamp = super::utils::file_modified_timestamp_ms(path);
    let fallback_session_id = extract_fallback_session_id(path);
    let sidecar_cwd = read_sidecar_workspace_cwd(path);

    let mut session_id = fallback_session_id;
    let mut cwd = sidecar_cwd;
    let mut current_model: Option<String> = None;
    let mut first_event_type: Option<String> = None;
    let mut usage_records: Vec<UnifiedMessage> = Vec::new();
    let mut shutdowns: Vec<ShutdownUsage> = Vec::new();
    let mut first_usage_timestamp: Option<i64> = None;

    let reader = BufReader::new(file);
    for (line_idx, line) in lossy_lines(reader).enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let Ok(event) = serde_json::from_str::<Value>(trimmed) else {
            // Tolerate truncated or partially written lines in active files.
            continue;
        };

        let Some(event_type) = event.get("type").and_then(Value::as_str) else {
            continue;
        };

        if first_event_type.is_none() {
            first_event_type = Some(event_type.to_string());
        }

        match event_type {
            "session.start" => {
                if let Some(sid) = event
                    .pointer("/data/sessionId")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    session_id = sid.to_string();
                }
                if cwd.is_none() {
                    cwd = event
                        .pointer("/data/context/cwd")
                        .or_else(|| event.pointer("/data/context/gitRoot"))
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string);
                }
            }
            "session.model_change" => {
                if let Some(new_model) = event
                    .pointer("/data/newModel")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|m| !m.is_empty() && *m != "auto")
                {
                    current_model = Some(new_model.to_string());
                }
            }
            "session.usage_record" => {
                if let Some(msg) = parse_usage_record(
                    &event,
                    &session_id,
                    current_model.as_deref(),
                    cwd.as_deref(),
                    fallback_timestamp,
                    line_idx,
                ) {
                    let ts = msg.timestamp;
                    first_usage_timestamp =
                        Some(first_usage_timestamp.map_or(ts, |existing| existing.min(ts)));
                    usage_records.push(msg);
                }
            }
            "session.shutdown" => {
                collect_shutdown_usage(&event, &mut shutdowns);
            }
            _ => {}
        }
    }

    let workspace_key = cwd.as_deref().and_then(normalize_workspace_key);
    let workspace_label = workspace_key.as_deref().and_then(workspace_label_from_key);

    if !usage_records.is_empty() {
        // If there were shutdowns timestamped strictly before the first usage_record,
        // differencing them accounts for any prefix history recorded before per-request
        // accounting began (e.g. upgraded sessions).
        if let Some(cutoff) = first_usage_timestamp {
            let prefix_shutdowns: Vec<ShutdownUsage> = shutdowns
                .into_iter()
                .filter(|s| s.timestamp_ms < cutoff)
                .collect();
            if !prefix_shutdowns.is_empty() {
                let complete = first_event_type.as_deref() == Some("session.start");
                let attribution = shutdown_deltas(prefix_shutdowns, complete);
                for delta in attribution.deltas {
                    let msg = shutdown_to_message(
                        &session_id,
                        workspace_key.as_deref(),
                        workspace_label.as_deref(),
                        delta,
                    );
                    if let Some(msg) = msg {
                        sink(path, msg);
                    }
                }
            }
        }

        for mut msg in usage_records {
            if msg.workspace_key.is_none() && workspace_key.is_some() {
                msg.set_workspace(workspace_key.as_deref(), workspace_label.as_deref());
            }
            sink(path, msg);
        }
    } else if !shutdowns.is_empty() {
        // No per-request records found: fall back to shutdown snapshots so older CLI
        // or shutdown-only sessions still contribute their tokens.
        let complete = first_event_type.as_deref() == Some("session.start");
        let attribution = shutdown_deltas(shutdowns, complete);
        for delta in attribution.deltas {
            let msg = shutdown_to_message(
                &session_id,
                workspace_key.as_deref(),
                workspace_label.as_deref(),
                delta,
            );
            if let Some(msg) = msg {
                sink(path, msg);
            }
        }
    }
}

fn parse_usage_record(
    event: &Value,
    session_id: &str,
    current_model: Option<&str>,
    cwd: Option<&str>,
    fallback_timestamp: i64,
    line_idx: usize,
) -> Option<UnifiedMessage> {
    let usage = event
        .get("data")
        .and_then(|d| d.get("usage"))
        .unwrap_or(event);

    let model_id = usage
        .get("model")
        .or_else(|| usage.get("copilot_usage_model"))
        .or_else(|| usage.get("modelDisplayName"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .or(current_model)
        .unwrap_or("unknown");
    let provider_id = inferred_provider_from_model(model_id).unwrap_or("github-copilot");

    let input_tokens = usage
        .get("inputTokens")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let output_tokens = usage
        .get("outputTokens")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let cache_read_tokens = usage
        .get("cacheReadTokens")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let cache_write_tokens = usage
        .get("cacheWriteTokens")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let reasoning_tokens = usage
        .get("reasoningTokens")
        .and_then(Value::as_i64)
        .unwrap_or(0);

    let cache_read = cache_read_tokens.max(0);
    let cache_write = cache_write_tokens.max(0);
    let tokens = TokenBreakdown {
        input: input_tokens
            .saturating_sub(cache_read)
            .saturating_sub(cache_write)
            .max(0),
        output: output_tokens.max(0),
        cache_read,
        cache_write,
        cache_write_1h: 0,
        reasoning: reasoning_tokens.max(0),
    };

    let total_nano_aiu = usage
        .pointer("/copilotUsage/totalNanoAiu")
        .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)))
        .unwrap_or(0)
        .max(0);

    if tokens.total() == 0 && total_nano_aiu == 0 {
        return None;
    }

    let timestamp = event
        .get("timestamp")
        .and_then(Value::as_str)
        .and_then(parse_timestamp_str)
        .unwrap_or(fallback_timestamp);

    // Copilot's `cost` field is the request multiplier (quota weight), NOT USD.
    // USD is computed exclusively from `totalNanoAiu` when present.
    let cost = if total_nano_aiu > 0 {
        total_nano_aiu as f64 / NANO_AIU_PER_USD
    } else {
        0.0
    };

    let event_id = event
        .get("id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("line-{line_idx}"));

    let dedup_key = format!("copilot-session-event:{}:{}", session_id, event_id);

    let mut message = UnifiedMessage::new_with_dedup(
        "copilot",
        model_id,
        provider_id,
        session_id,
        timestamp,
        tokens,
        cost,
        Some(dedup_key),
    );

    message.duration_ms = usage.get("duration").and_then(Value::as_i64);

    if total_nano_aiu > 0 {
        message.mark_provider_reported_cost();
    }

    if let Some(workspace_key) = cwd.and_then(normalize_workspace_key) {
        let workspace_label = workspace_label_from_key(&workspace_key);
        message.set_workspace(Some(workspace_key), workspace_label);
    }

    Some(message)
}

fn shutdown_to_message(
    session_id: &str,
    workspace_key: Option<&str>,
    workspace_label: Option<&str>,
    delta: ShutdownUsage,
) -> Option<UnifiedMessage> {
    let model_id = delta
        .attributed_model
        .as_deref()
        .filter(|model| !model.is_empty() && *model != "auto")
        .unwrap_or(&delta.model);
    let provider_id = inferred_provider_from_model(model_id).unwrap_or("github-copilot");

    let cache_read = delta.cache_read.max(0);
    let cache_write = delta.cache_write.max(0);
    let tokens = TokenBreakdown {
        input: delta
            .input
            .saturating_sub(cache_read)
            .saturating_sub(cache_write)
            .max(0),
        output: delta.output.max(0),
        cache_read,
        cache_write,
        cache_write_1h: 0,
        reasoning: delta.reasoning.max(0),
    };

    if tokens.total() == 0 {
        return None;
    }

    // The tracker (`modelMetrics`) counts `auto` as its own counter, so the
    // key must name the tracker model, not the resolved attribution — keying
    // by the resolved model collides a shutdown that carries both and drops
    // a row (mirrors the desktop parser's keying).
    let dedup_key = format!(
        "copilot-session-event:{}:shutdown:{}:{}",
        session_id, delta.event_id, delta.model
    );

    let mut message = UnifiedMessage::new_with_dedup(
        "copilot",
        model_id,
        provider_id,
        session_id,
        delta.timestamp_ms,
        tokens,
        0.0,
        Some(dedup_key),
    );

    if workspace_key.is_some() {
        message.set_workspace(workspace_key, workspace_label);
    }

    Some(message)
}

fn extract_fallback_session_id(path: &Path) -> String {
    if path.file_name().and_then(|n| n.to_str()) == Some("events.jsonl") {
        if let Some(parent) = path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
        {
            return parent.to_string();
        }
    }
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown-copilot-session")
        .to_string()
}

fn read_sidecar_workspace_cwd(path: &Path) -> Option<String> {
    let parent = path.parent()?;
    let workspace_file = parent.join("workspace.yaml");
    let content = std::fs::read_to_string(workspace_file).ok()?;
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(cwd) = trimmed.strip_prefix("cwd:") {
            let val = cwd.trim().trim_matches('"').trim_matches('\'');
            if !val.is_empty() {
                return Some(val.to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn turn_index_sorts_file_timestamps_and_applies_only_cross_lane_jitter() {
        let index = SessionStateTurnIndex::new(HashMap::from([
            ("session-a".into(), vec![3_000, 1_000, 1_000]),
            ("session-empty".into(), Vec::new()),
        ]));

        for timestamp in [980, 981, 1_000, 1_019, 1_020, 2_980, 3_020] {
            assert!(index.overlaps("session-a", timestamp), "{timestamp}");
        }
        for timestamp in [979, 1_021, 2_000, 2_979, 3_021] {
            assert!(!index.overlaps("session-a", timestamp), "{timestamp}");
        }
        assert!(!index.overlaps("another-session", 1_000));
        assert!(!index.overlaps("session-empty", 1_000));
        // Checking a retained DB row must not cause the next nearby DB row
        // to disappear when neither has a counterpart in the event file.
        assert!(!index.overlaps("session-a", 2_000));
        assert!(!index.overlaps("session-a", 2_010));
    }

    #[test]
    fn turn_index_handles_timestamp_extremes_without_overflow() {
        let index = SessionStateTurnIndex::new(HashMap::from([
            ("earliest".into(), vec![i64::MIN]),
            ("latest".into(), vec![i64::MAX]),
            ("pre-epoch".into(), vec![-1]),
        ]));

        assert!(index.overlaps("earliest", i64::MIN));
        assert!(index.overlaps("earliest", i64::MIN + 20));
        assert!(!index.overlaps("earliest", i64::MIN + 21));
        assert!(index.overlaps("latest", i64::MAX));
        assert!(index.overlaps("latest", i64::MAX - 20));
        assert!(!index.overlaps("latest", i64::MAX - 21));
        assert!(!index.overlaps("earliest", i64::MAX));
        assert!(!index.overlaps("latest", i64::MIN));
        assert!(!index.overlaps("pre-epoch", i64::MAX));
    }

    #[test]
    fn parses_usage_record_event_file_with_exact_fields() {
        let dir = tempdir().unwrap();
        let session_dir = dir.path().join("c79fb3cc-c0ef-4d0c-b212-07c79099fe36");
        std::fs::create_dir_all(&session_dir).unwrap();
        let events_path = session_dir.join("events.jsonl");

        let content = r#"{"type":"session.start","data":{"sessionId":"c79fb3cc-c0ef-4d0c-b212-07c79099fe36","context":{"cwd":"/Users/dev/myrepo"}},"id":"start-1","timestamp":"2026-11-02T10:00:00.000Z"}
{"type":"session.usage_record","data":{"usage":{"model":"gpt-6.1-sol","inputTokens":32832,"outputTokens":32,"cacheReadTokens":0,"cacheWriteTokens":32829,"reasoningTokens":0,"cost":1.0,"duration":1500,"copilotUsage":{"totalNanoAiu":181977250}}},"id":"ev-1","timestamp":"2026-11-02T10:01:00.000Z"}
{"type":"session.usage_record","data":{"usage":{"model":"gpt-6-luna","inputTokens":21363,"outputTokens":179,"cacheReadTokens":20623,"cacheWriteTokens":737,"reasoningTokens":119,"cost":1.0,"duration":2799,"copilotUsage":{"totalNanoAiu":27170850}}},"id":"ev-2","timestamp":"2026-11-02T10:02:00.000Z"}
"#;
        std::fs::write(&events_path, content).unwrap();

        let msgs = parse_copilot_session_state_file(&events_path);
        assert_eq!(msgs.len(), 2);

        // Record 1: gpt-6.1-sol
        assert_eq!(msgs[0].session_id, "c79fb3cc-c0ef-4d0c-b212-07c79099fe36");
        assert_eq!(msgs[0].model_id, "gpt-6.1-sol");
        assert_eq!(msgs[0].provider_id, "openai");
        assert_eq!(msgs[0].tokens.input, 3); // 32832 - 0 - 32829
        assert_eq!(msgs[0].tokens.output, 32);
        assert_eq!(msgs[0].tokens.cache_read, 0);
        assert_eq!(msgs[0].tokens.cache_write, 32829);
        assert_eq!(msgs[0].duration_ms, Some(1500));
        assert!((msgs[0].cost - 0.0018197725).abs() < 1e-10);
        assert_eq!(msgs[0].cost_source, crate::CostSource::ProviderReported);
        assert_eq!(msgs[0].workspace_label.as_deref(), Some("myrepo"));
        assert_eq!(
            msgs[0].dedup_key.as_deref(),
            Some("copilot-session-event:c79fb3cc-c0ef-4d0c-b212-07c79099fe36:ev-1")
        );

        // Record 2: gpt-6-luna
        assert_eq!(msgs[1].model_id, "gpt-6-luna");
        assert_eq!(msgs[1].tokens.input, 3); // 21363 - 20623 - 737
        assert_eq!(msgs[1].tokens.output, 179);
        assert_eq!(msgs[1].tokens.reasoning, 119);
        assert_eq!(msgs[1].tokens.cache_read, 20623);
        assert_eq!(msgs[1].tokens.cache_write, 737);
    }

    #[test]
    fn falls_back_to_shutdown_deltas_when_no_usage_records() {
        let dir = tempdir().unwrap();
        let session_dir = dir.path().join("sess-legacy");
        std::fs::create_dir_all(&session_dir).unwrap();
        let events_path = session_dir.join("events.jsonl");

        let content = r#"{"type":"session.start","data":{"sessionId":"sess-legacy","context":{"cwd":"/Users/dev/proj"}},"id":"start-1","timestamp":"2026-10-01T10:00:00.000Z"}
{"type":"session.shutdown","data":{"modelMetrics":{"gpt-6.1-sol":{"usage":{"inputTokens":1000,"outputTokens":100,"cacheReadTokens":500,"cacheWriteTokens":0,"reasoningTokens":10}}}},"id":"shut-1","timestamp":"2026-10-01T10:10:00.000Z"}
"#;
        std::fs::write(&events_path, content).unwrap();

        let msgs = parse_copilot_session_state_file(&events_path);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].session_id, "sess-legacy");
        assert_eq!(msgs[0].model_id, "gpt-6.1-sol");
        assert_eq!(msgs[0].tokens.input, 500); // 1000 - 500
        assert_eq!(msgs[0].tokens.output, 100);
        assert_eq!(msgs[0].tokens.cache_read, 500);
        assert_eq!(msgs[0].tokens.reasoning, 10);
    }

    /// A shutdown under an `auto` tracker key that also names the concrete
    /// attributed model must be keyed by the tracker model — the desktop
    /// parser convention. Keying by the resolved model would collide the two
    /// counter identities and drop a row.
    #[test]
    fn shutdown_dedup_key_names_the_tracker_model_not_the_resolved_one() {
        let message = shutdown_to_message(
            "sess-auto",
            None,
            None,
            ShutdownUsage {
                event_id: "shut-auto-1".to_string(),
                timestamp_ms: 1_762_000_000_000,
                model: "auto".to_string(),
                attributed_model: Some("gpt-5.2".to_string()),
                input: 100,
                output: 10,
                cache_read: 0,
                cache_write: 0,
                reasoning: 0,
            },
        )
        .expect("the shutdown carries usage");

        assert_eq!(message.model_id, "gpt-5.2");
        assert_eq!(
            message.dedup_key.as_deref(),
            Some("copilot-session-event:sess-auto:shutdown:shut-auto-1:auto")
        );
    }

    #[test]
    fn handles_active_session_with_trailing_incomplete_line() {
        let dir = tempdir().unwrap();
        let session_dir = dir.path().join("sess-active");
        std::fs::create_dir_all(&session_dir).unwrap();
        let events_path = session_dir.join("events.jsonl");

        let content = r#"{"type":"session.start","data":{"sessionId":"sess-active","context":{"cwd":"/Users/dev/proj"}},"id":"start-1","timestamp":"2026-11-01T10:00:00.000Z"}
{"type":"session.usage_record","data":{"usage":{"model":"gpt-6.1-sol","inputTokens":100,"outputTokens":20,"cacheReadTokens":0,"cacheWriteTokens":0,"reasoningTokens":0}},"id":"ev-1","timestamp":"2026-11-01T10:01:00.000Z"}
{"type":"session.usage_record","data":{"usage":{"model":"gpt-6.1-sol","inputTokens":20
"#;
        std::fs::write(&events_path, content).unwrap();

        let msgs = parse_copilot_session_state_file(&events_path);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].tokens.input, 100);
    }

    #[test]
    fn ignores_cost_multiplier_when_nano_aiu_is_absent() {
        let dir = tempdir().unwrap();
        let session_dir = dir.path().join("sess-no-aiu");
        std::fs::create_dir_all(&session_dir).unwrap();
        let events_path = session_dir.join("events.jsonl");

        let content = r#"{"type":"session.start","data":{"sessionId":"sess-no-aiu"},"id":"start-1","timestamp":"2026-11-01T10:00:00.000Z"}
{"type":"session.usage_record","data":{"usage":{"model":"gpt-6.1-sol","inputTokens":100,"outputTokens":20,"cacheReadTokens":0,"cacheWriteTokens":0,"reasoningTokens":0,"cost":14.0}},"id":"ev-1","timestamp":"2026-11-01T10:01:00.000Z"}
"#;
        std::fs::write(&events_path, content).unwrap();

        let msgs = parse_copilot_session_state_file(&events_path);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].cost, 0.0);
        assert_eq!(msgs[0].cost_source, crate::CostSource::Unknown);
    }
}
