//! fx (fx.sh) session parser.
//!
//! fx stores interactive sessions under `~/.fx/sessions/<session-id>/`.
//! Each session directory contains a `usage-v2.json` file with a cumulative
//! usage snapshot, including per-model token breakdowns and cost.
//!
//! The snapshot is cumulative — it reflects the session's total usage at the
//! time of the last checkpoint — so one `usage-v2.json` produces one
//! [`UnifiedMessage`] per model, timestamped at the session's `updated_at_ms`.
//!
//! fx also writes an `events.jsonl` with `usage_checkpointed` events that
//! carry per-call deltas. This parser uses the simpler snapshot approach:
//! the total is what matters for cost tracking, and the snapshot is the
//! authoritative final value.

use super::UnifiedMessage;
use crate::TokenBreakdown;
use serde::Deserialize;
use std::path::Path;

const CLIENT_ID: &str = "fx";

/// Top-level `usage-v2.json` structure.
#[derive(Debug, Deserialize)]
struct FxUsageFile {
    #[serde(default)]
    session_id: Option<String>,
    snapshot: FxUsageSnapshot,
}

#[derive(Debug, Deserialize)]
struct FxUsageSnapshot {
    #[serde(default)]
    total_cost: f64,
    #[serde(default)]
    input_tokens: i64,
    #[serde(default)]
    output_tokens: i64,
    #[serde(default)]
    cache_read_tokens: i64,
    #[serde(default)]
    cache_write_tokens: i64,
    #[serde(default)]
    reasoning_tokens: i64,
    #[serde(default)]
    request_count: i64,
    #[serde(default)]
    api_duration_ms: i64,
    #[serde(default)]
    models: Vec<FxModelUsage>,
}

#[derive(Debug, Deserialize)]
struct FxModelUsage {
    model: String,
    #[serde(default)]
    total_cost: f64,
    #[serde(default)]
    input_tokens: i64,
    #[serde(default)]
    output_tokens: i64,
    #[serde(default)]
    cache_read_tokens: i64,
    #[serde(default)]
    cache_write_tokens: i64,
    #[serde(default)]
    reasoning_tokens: i64,
    #[serde(default)]
    request_count: i64,
}

/// `session.json` structure — used to extract the session ID, workspace, and
/// timestamp when the usage file doesn't carry them or when we need the
/// session's `updated_at_ms` as the message timestamp.
#[derive(Debug, Deserialize)]
struct FxSessionFile {
    id: Option<String>,
    #[serde(default)]
    created_at_ms: i64,
    #[serde(default)]
    updated_at_ms: i64,
    #[serde(default)]
    workspace_root: Option<String>,
}

/// Parse a `usage-v2.json` file from an fx session directory.
///
/// The path is expected to be `<session-dir>/usage-v2.json`. The sibling
/// `session.json` is read for the session ID, workspace, and timestamp when
/// available.
pub fn parse_fx_file(path: &Path) -> Vec<UnifiedMessage> {
    let data = match std::fs::read(path) {
        Ok(data) => data,
        Err(_) => return Vec::new(),
    };
    let mut bytes = data;
    let usage_file: FxUsageFile = match simd_json::from_slice(&mut bytes) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };

    let session_dir = path.parent();

    // Try to read sibling session.json for metadata.
    let session_meta = session_dir.and_then(|dir| {
        let session_path = dir.join("session.json");
        let session_data = std::fs::read(&session_path).ok()?;
        let mut session_bytes = session_data;
        simd_json::from_slice::<FxSessionFile>(&mut session_bytes).ok()
    });

    let session_id = usage_file
        .session_id
        .clone()
        .or_else(|| session_meta.as_ref().and_then(|s| s.id.clone()))
        .unwrap_or_else(|| {
            session_dir
                .and_then(|d| d.file_name())
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string()
        });

    let timestamp = session_meta
        .as_ref()
        .map(|s| {
            let updated = s.updated_at_ms;
            if updated > 0 {
                updated
            } else {
                s.created_at_ms
            }
        })
        .filter(|&t| t > 0)
        .or_else(|| {
            // Fall back to the usage file's mtime.
            session_dir.and_then(|_| {
                std::fs::metadata(path)
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_millis() as i64)
            })
        })
        .unwrap_or(0);

    let workspace_root = session_meta.as_ref().and_then(|s| s.workspace_root.clone());

    let (workspace_key, workspace_label) = workspace_root
        .as_deref()
        .and_then(|ws| {
            let key = super::normalize_workspace_key(ws)?;
            let label = super::workspace_label_from_key(&key);
            Some((Some(key), label))
        })
        .unwrap_or((None, None));

    let snapshot = &usage_file.snapshot;

    // If the snapshot has per-model breakdowns, emit one message per model.
    if !snapshot.models.is_empty() {
        return snapshot
            .models
            .iter()
            .filter_map(|model| model_to_message(model, &session_id, timestamp))
            .map(|mut msg| {
                msg.set_workspace(workspace_key.clone(), workspace_label.clone());
                msg
            })
            .collect();
    }

    // Fallback: emit a single aggregate message from the snapshot totals.
    let tokens = TokenBreakdown {
        input: snapshot.input_tokens.max(0),
        output: snapshot.output_tokens.max(0),
        cache_read: snapshot.cache_read_tokens.max(0),
        cache_write: snapshot.cache_write_tokens.max(0),
        reasoning: snapshot.reasoning_tokens.max(0),
    };

    if tokens.total() == 0 && snapshot.total_cost == 0.0 {
        return Vec::new();
    }

    let message_count = snapshot.request_count.max(0) as i32;
    let duration_ms = if snapshot.api_duration_ms > 0 {
        Some(snapshot.api_duration_ms)
    } else {
        None
    };

    let mut message = UnifiedMessage::new_with_dedup(
        CLIENT_ID,
        "unknown",
        "unknown",
        session_id.as_str(),
        timestamp,
        tokens,
        snapshot.total_cost,
        Some(format!("fx:{session_id}")),
    );
    message.message_count = message_count.max(1);
    message.duration_ms = duration_ms;
    message.set_workspace(workspace_key, workspace_label);

    vec![message]
}

fn model_to_message(
    model: &FxModelUsage,
    session_id: &str,
    timestamp: i64,
) -> Option<UnifiedMessage> {
    let tokens = TokenBreakdown {
        input: model.input_tokens.max(0),
        output: model.output_tokens.max(0),
        cache_read: model.cache_read_tokens.max(0),
        cache_write: model.cache_write_tokens.max(0),
        reasoning: model.reasoning_tokens.max(0),
    };

    // Skip a model entry with zero usage and zero cost.
    if tokens.total() == 0 && model.total_cost == 0.0 {
        return None;
    }

    // fx model ids use the `provider/model` convention (e.g. "zai/glm-5.2").
    let (provider_id, model_id) = split_provider_model(&model.model);

    let message_count = model.request_count.max(0) as i32;

    let mut message = UnifiedMessage::new_with_dedup(
        CLIENT_ID,
        model_id,
        provider_id,
        session_id,
        timestamp,
        tokens,
        model.total_cost,
        Some(format!("fx:{session_id}:{}", model.model)),
    );
    message.message_count = message_count.max(1);
    Some(message)
}

/// Split a `provider/model` id into `(provider, model)`. If there is no slash,
/// the whole string is the model id and the provider is "unknown".
fn split_provider_model(id: &str) -> (String, String) {
    match id.split_once('/') {
        Some((provider, model)) => {
            let provider = if provider.is_empty() {
                "unknown".to_string()
            } else {
                provider.to_string()
            };
            let model = if model.is_empty() {
                "unknown".to_string()
            } else {
                model.to_string()
            };
            (provider, model)
        }
        None => ("unknown".to_string(), id.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_usage_v2_with_per_model_breakdown() {
        let dir = tempfile::TempDir::new().unwrap();
        let session_dir = dir.path().join("1787157077624-1787157077624725000-abc123");
        std::fs::create_dir_all(&session_dir).unwrap();

        std::fs::write(
            session_dir.join("session.json"),
            r#"{"id":"1787157077624-1787157077624725000-abc123","created_at_ms":1787157077624,"updated_at_ms":1787157583543,"workspace_root":"/Users/test/project"}"#,
        )
        .unwrap();

        std::fs::write(
            session_dir.join("usage-v2.json"),
            r#"{"schema_version":1,"session_id":"1787157077624-1787157077624725000-abc123","snapshot":{"schema_version":2,"total_cost":0.005,"input_tokens":167841,"output_tokens":2434,"cache_read_tokens":153408,"cache_write_tokens":0,"reasoning_tokens":0,"request_count":9,"api_duration_ms":206154,"models":[{"model":"zai/glm-5.2","total_cost":0.005,"input_tokens":167841,"output_tokens":2434,"cache_read_tokens":153408,"cache_write_tokens":0,"reasoning_tokens":0,"request_count":9}]}}"#,
        )
        .unwrap();

        let messages = parse_fx_file(&session_dir.join("usage-v2.json"));
        assert_eq!(messages.len(), 1);

        let msg = &messages[0];
        assert_eq!(msg.client, "fx");
        assert_eq!(msg.model_id, "glm-5.2");
        assert_eq!(msg.provider_id, "zai");
        assert_eq!(msg.session_id, "1787157077624-1787157077624725000-abc123");
        assert_eq!(msg.timestamp, 1787157583543);
        assert_eq!(msg.tokens.input, 167841);
        assert_eq!(msg.tokens.output, 2434);
        assert_eq!(msg.tokens.cache_read, 153408);
        assert_eq!(msg.tokens.cache_write, 0);
        assert_eq!(msg.tokens.reasoning, 0);
        assert_eq!(msg.cost, 0.005);
        assert_eq!(msg.message_count, 9);
        assert!(msg.workspace_key.is_some());
        assert!(msg.workspace_label.is_some());
    }

    #[test]
    fn parses_usage_v2_without_models() {
        let dir = tempfile::TempDir::new().unwrap();
        let session_dir = dir.path().join("session-without-models");
        std::fs::create_dir_all(&session_dir).unwrap();

        std::fs::write(
            session_dir.join("session.json"),
            r#"{"id":"sess-2","created_at_ms":1000,"updated_at_ms":2000}"#,
        )
        .unwrap();

        std::fs::write(
            session_dir.join("usage-v2.json"),
            r#"{"session_id":"sess-2","snapshot":{"total_cost":0.01,"input_tokens":500,"output_tokens":100,"cache_read_tokens":200,"cache_write_tokens":0,"reasoning_tokens":0,"request_count":3,"api_duration_ms":5000,"models":[]}}"#,
        )
        .unwrap();

        let messages = parse_fx_file(&session_dir.join("usage-v2.json"));
        assert_eq!(messages.len(), 1);

        let msg = &messages[0];
        assert_eq!(msg.client, "fx");
        assert_eq!(msg.model_id, "unknown");
        assert_eq!(msg.tokens.input, 500);
        assert_eq!(msg.tokens.output, 100);
        assert_eq!(msg.cost, 0.01);
        assert_eq!(msg.message_count, 3);
    }

    #[test]
    fn skips_zero_usage_snapshot() {
        let dir = tempfile::TempDir::new().unwrap();
        let session_dir = dir.path().join("empty-session");
        std::fs::create_dir_all(&session_dir).unwrap();

        std::fs::write(
            session_dir.join("session.json"),
            r#"{"id":"sess-3","created_at_ms":1000,"updated_at_ms":2000}"#,
        )
        .unwrap();

        std::fs::write(
            session_dir.join("usage-v2.json"),
            r#"{"session_id":"sess-3","snapshot":{"total_cost":0,"input_tokens":0,"output_tokens":0,"cache_read_tokens":0,"cache_write_tokens":0,"reasoning_tokens":0,"request_count":0,"models":[]}}"#,
        )
        .unwrap();

        let messages = parse_fx_file(&session_dir.join("usage-v2.json"));
        assert!(messages.is_empty());
    }

    #[test]
    fn derives_session_id_from_directory_name() {
        let dir = tempfile::TempDir::new().unwrap();
        let session_dir = dir.path().join("1787157077624-abc");
        std::fs::create_dir_all(&session_dir).unwrap();

        // No session.json — session id should come from the directory name.
        std::fs::write(
            session_dir.join("usage-v2.json"),
            r#"{"snapshot":{"total_cost":0.01,"input_tokens":100,"output_tokens":50,"models":[]}}"#,
        )
        .unwrap();

        let messages = parse_fx_file(&session_dir.join("usage-v2.json"));
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].session_id, "1787157077624-abc");
    }

    #[test]
    fn skips_model_with_zero_usage() {
        let dir = tempfile::TempDir::new().unwrap();
        let session_dir = dir.path().join("multi-model");
        std::fs::create_dir_all(&session_dir).unwrap();

        std::fs::write(
            session_dir.join("session.json"),
            r#"{"id":"multi","created_at_ms":1000,"updated_at_ms":2000}"#,
        )
        .unwrap();

        std::fs::write(
            session_dir.join("usage-v2.json"),
            r#"{"session_id":"multi","snapshot":{"total_cost":0.005,"input_tokens":1000,"output_tokens":500,"models":[{"model":"zai/glm-5.2","total_cost":0.005,"input_tokens":1000,"output_tokens":500,"request_count":5},{"model":"openai/gpt-5.4","total_cost":0,"input_tokens":0,"output_tokens":0,"request_count":0}]}}"#,
        )
        .unwrap();

        let messages = parse_fx_file(&session_dir.join("usage-v2.json"));
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "glm-5.2");
    }

    #[test]
    fn handles_missing_usage_file() {
        let result = parse_fx_file(Path::new("/nonexistent/usage-v2.json"));
        assert!(result.is_empty());
    }
}
