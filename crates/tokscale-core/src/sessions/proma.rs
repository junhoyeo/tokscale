//! Proma's persisted Agent usage: native Pi sessions and legacy Claude SDK
//! transcripts, with the UI journal used only where the native record is absent.
//! See docs/proma.md for the pinned upstream writer contracts and precedence.
use super::pi::{self, PiFormatObserver, PiSessionEntry};
use super::utils::{
    extract_cache_write_1h, file_modified_timestamp_ms, for_each_json_line,
    for_each_json_line_with_bytes, parse_timestamp_value,
};
use super::{normalize_workspace_key, workspace_label_from_key, UnifiedMessage};
use crate::provider_identity::inferred_provider_from_model;
use crate::TokenBreakdown;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};

// This envelope is a per-record identity, not a reconciliation decision. It is
// cached with the raw file so adding/removing a native session or updating the
// index takes effect even when every transcript is a warm cache hit.
#[derive(Serialize, Deserialize)]
enum Identity {
    Pi {
        entry: Option<String>,
        key: Option<String>,
    },
    Sdk {
        uuid: Option<String>,
        response: Option<String>,
    },
}

fn text(value: &Value, field: &str) -> Option<String> {
    value
        .get(field)?
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.contains('\u{fffd}'))
        .map(str::to_owned)
}

/// Only Agent journals and the two SDK trees are usage sources. In particular,
/// conversation exports, channel credentials and workspace files are not logs.
pub(crate) fn session_roots(root: &Path) -> Vec<PathBuf> {
    if root.file_name().is_some_and(|n| n == "agent-sessions")
        || (root
            .file_name()
            .is_some_and(|n| n == "sessions" || n == "projects")
            && root
                .parent()
                .and_then(Path::file_name)
                .is_some_and(|n| n == "sdk-config"))
    {
        return vec![root.to_path_buf()];
    }
    [
        "agent-sessions",
        "sdk-config/sessions",
        "sdk-config/projects",
    ]
    .map(|p| root.join(p))
    .to_vec()
}

fn config_root(path: &Path) -> Option<PathBuf> {
    path.ancestors()
        .find(|p| {
            p.file_name()
                .is_some_and(|n| n == "agent-sessions" || n == "sdk-config")
        })?
        .parent()
        .map(Path::to_path_buf)
}

#[derive(Default)]
struct PiObserver {
    identities: Vec<(Option<String>, Option<f64>)>,
}
impl PiFormatObserver for PiObserver {
    fn observe_entry(&mut self, entry: &PiSessionEntry, emitted: Option<&UnifiedMessage>) {
        if emitted.is_some() {
            let usage = entry
                .message
                .as_ref()
                .and_then(|m| m.usage.as_ref())
                .or(entry.usage.as_ref());
            let cost = usage
                .and_then(|u| u.cost.as_ref())
                .and_then(|c| c.total)
                .filter(|c| c.is_finite() && *c >= 0.0);
            self.identities.push((entry.id.clone(), cost));
        }
    }
}

pub fn parse_proma_file(path: &Path) -> Vec<UnifiedMessage> {
    let mut is_pi = false;
    for_each_json_line_with_bytes(path, &mut |line| {
        if let Ok(record) = serde_json::from_str::<Value>(line.trimmed) {
            is_pi = record.get("type").and_then(Value::as_str) == Some("session");
            return ControlFlow::Break(());
        }
        ControlFlow::Continue(())
    });
    if is_pi {
        let mut observer = PiObserver::default();
        let mut messages =
            pi::parse_pi_format_file_with_observer(path, "proma", "proma", &mut observer);
        for (message, (entry, cost)) in messages.iter_mut().zip(observer.identities) {
            let key = message.dedup_key.take();
            message.dedup_key = serde_json::to_string(&Identity::Pi { entry, key }).ok();
            // Proma persists Pi's per-call USD cost even for custom channels.
            // An explicit zero is authoritative; missing cost remains estimable.
            if let Some(cost) = cost {
                message.cost = cost;
                message.mark_provider_reported_cost();
            }
        }
        return messages;
    }
    parse_sdk_file(path)
}

fn parse_sdk_file(path: &Path) -> Vec<UnifiedMessage> {
    let fallback = file_modified_timestamp_ms(path);
    let fallback_session = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown");
    let mut messages = Vec::new();
    for_each_json_line(path, &mut |_, line| {
        let Ok(record) = serde_json::from_str::<Value>(line) else {
            return;
        };
        // Result usage is a session/turn aggregate, not another API request.
        if record.get("type").and_then(Value::as_str) != Some("assistant") {
            return;
        }
        let Some(body) = record.get("message") else {
            return;
        };
        let Some(usage) = body.get("usage").filter(|v| v.is_object()) else {
            return;
        };
        let model = text(body, "model").unwrap_or_else(|| "unknown".into());
        if model == "<synthetic>" {
            return;
        }
        let counter = |key| usage.get(key).and_then(Value::as_i64).unwrap_or(0).max(0);
        let cache_write = counter("cache_creation_input_tokens");
        let tokens = TokenBreakdown {
            input: counter("input_tokens"),
            output: counter("output_tokens"),
            cache_read: counter("cache_read_input_tokens"),
            cache_write,
            cache_write_1h: extract_cache_write_1h(usage).min(cache_write),
            reasoning: 0,
        };
        if tokens.total() == 0 {
            return;
        }
        let timestamp = record
            .get("_createdAt")
            .and_then(parse_timestamp_value)
            .or_else(|| record.get("timestamp").and_then(parse_timestamp_value))
            .unwrap_or(fallback);
        let session = text(&record, "session_id")
            .or_else(|| text(&record, "sessionId"))
            .unwrap_or_else(|| fallback_session.to_owned());
        let provider = inferred_provider_from_model(&model).unwrap_or("proma");
        let mut message =
            UnifiedMessage::new("proma", &model, provider, session, timestamp, tokens, 0.0);
        if let Some(cwd) = text(&record, "cwd") {
            let key = normalize_workspace_key(&cwd);
            let label = key.as_deref().and_then(workspace_label_from_key);
            message.set_workspace(key, label);
        }
        message.service_tier = text(body, "service_tier");
        message.dedup_key = serde_json::to_string(&Identity::Sdk {
            uuid: text(&record, "uuid"),
            response: text(body, "id"),
        })
        .ok();
        messages.push(message);
    });
    messages
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionMeta {
    id: String,
    sdk_session_id: Option<String>,
    pi_session_file: Option<String>,
    #[serde(default)]
    pi_entry_bindings: HashMap<String, String>,
}
#[derive(Default, Deserialize)]
struct Index {
    #[serde(default)]
    sessions: Vec<SessionMeta>,
}

fn read_index(root: &Path) -> Index {
    let path = root.join("agent-sessions.json");
    // Never follow a metadata link or open the arbitrary paths in its content.
    const LIMIT: u64 = 16 * 1024 * 1024;
    let Ok(meta) = std::fs::symlink_metadata(&path) else {
        return Index::default();
    };
    if !meta.is_file() || meta.len() > LIMIT {
        return Index::default();
    }
    let Ok(file) = std::fs::File::open(path) else {
        return Index::default();
    };
    let mut bytes = Vec::new();
    if file.take(LIMIT + 1).read_to_end(&mut bytes).is_err() || bytes.len() as u64 > LIMIT {
        return Index::default();
    }
    serde_json::from_slice(&bytes).unwrap_or_default()
}

/// Reconcile after loading raw source caches. Native Pi files take precedence
/// over their SDK-shaped display mirrors; stable response/entry IDs also remove
/// copied fork history. No content or token-count fingerprint is an identity.
pub(crate) fn reconcile_sources(
    mut sources: Vec<(PathBuf, Vec<UnifiedMessage>)>,
) -> Vec<UnifiedMessage> {
    sources.sort_by(|a, b| {
        let display = |p: &Path| {
            p.parent()
                .and_then(Path::file_name)
                .is_some_and(|n| n == "agent-sessions")
        };
        (display(&a.0), &a.0).cmp(&(display(&b.0), &b.0))
    });
    let mut indexes = HashMap::new();
    let mut pi_sessions = HashSet::new();
    let mut pi_entries = HashSet::new();
    let mut path_sessions: HashMap<PathBuf, String> = HashMap::new();
    let mut uuid_responses = HashMap::new();
    for (path, messages) in &sources {
        if let Some(root) = config_root(path) {
            indexes
                .entry(root.clone())
                .or_insert_with(|| read_index(&root));
        }
        for message in messages {
            match identity(message) {
                Some(Identity::Pi { entry, .. }) => {
                    pi_sessions.insert(message.session_id.clone());
                    path_sessions.insert(path.clone(), message.session_id.clone());
                    if let Some(entry) = entry {
                        pi_entries.insert((message.session_id.clone(), entry));
                    }
                }
                Some(Identity::Sdk {
                    uuid: Some(uuid),
                    response: Some(response),
                }) => {
                    uuid_responses.insert(uuid, response);
                }
                _ => {}
            }
        }
    }
    let mut output: Vec<UnifiedMessage> = Vec::new();
    let mut seen: HashMap<String, usize> = HashMap::new();
    for (path, messages) in sources {
        let meta = config_root(&path)
            .and_then(|root| indexes.get(&root))
            .and_then(|index| {
                index
                    .sessions
                    .iter()
                    .find(|m| Some(m.id.as_str()) == path.file_stem().and_then(|s| s.to_str()))
            });
        let linked_session = meta.and_then(|m| {
            m.pi_session_file
                .as_ref()
                .and_then(|p| path_sessions.get(Path::new(p)))
                .or(m
                    .sdk_session_id
                    .as_ref()
                    .filter(|s| pi_sessions.contains(*s)))
        });
        for mut message in messages {
            let key = match identity(&message) {
                Some(Identity::Pi { key, .. }) => key,
                Some(Identity::Sdk { uuid, response }) => {
                    // Pi's adapter omits message.id. Claude SDK records retain
                    // it, including legacy history copied into a Pi UI session.
                    if response.is_none() {
                        let binding = uuid
                            .as_ref()
                            .and_then(|uuid| meta.and_then(|m| m.pi_entry_bindings.get(uuid)));
                        let native =
                            linked_session.or_else(|| pi_sessions.get(&message.session_id));
                        if let Some(session) = native {
                            // A known binding missing from the native file may
                            // be a crash-truncated tail: keep its display copy.
                            if binding.is_none_or(|entry| {
                                pi_entries.contains(&(session.clone(), entry.clone()))
                            }) {
                                continue;
                            }
                        }
                    }
                    response
                        .or_else(|| uuid.as_ref().and_then(|id| uuid_responses.get(id).cloned()))
                        .map(|id| format!("proma:claude:{id}"))
                        .or_else(|| uuid.map(|id| format!("proma:uuid:{id}")))
                }
                None => message.dedup_key.clone(),
            };
            message.dedup_key = key.clone();
            if let Some(key) = key {
                if let Some(&i) = seen.get(&key) {
                    // Streaming snapshots repeat a call. Retain its fullest
                    // measured snapshot, never sum it with an earlier prefix.
                    if message.tokens.total() > output[i].tokens.total()
                        || (message.tokens.total() == output[i].tokens.total()
                            && message.cost_source == super::CostSource::ProviderReported
                            && output[i].cost_source != super::CostSource::ProviderReported)
                    {
                        output[i] = message;
                    }
                    continue;
                }
                seen.insert(key, output.len());
            }
            output.push(message);
        }
    }
    output
}

fn identity(message: &UnifiedMessage) -> Option<Identity> {
    serde_json::from_str(message.dedup_key.as_deref()?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessions::CostSource;
    use serde_json::json;

    fn write(root: &Path, relative: &str, records: Vec<Value>) -> PathBuf {
        let path = root.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            records
                .iter()
                .map(Value::to_string)
                .collect::<Vec<_>>()
                .join("\n"),
        )
        .unwrap();
        path
    }
    fn sdk(uuid: &str, response: Option<&str>, session: &str, output: i64) -> Value {
        json!({"type":"assistant", "uuid":uuid, "session_id":session, "_createdAt":1778000000123i64,
            "message":{"id":response,"model":"claude-sonnet-4", "usage":{
                "input_tokens":100,"output_tokens":output,"cache_read_input_tokens":200,
                "cache_creation_input_tokens":30,"cache_creation":{"ephemeral_1h_input_tokens":20}}}})
    }
    fn pi_records(session: &str, entry: &str, cost: Option<f64>) -> Vec<Value> {
        vec![
            json!({"type":"session","version":3,"id":session,"timestamp":"2026-05-05T12:00:00Z","cwd":"/project"}),
            json!({"type":"message","id":entry,"timestamp":"2026-05-05T12:01:00Z","message":{
                "role":"assistant","model":"mimo-v2.5-pro","provider":"proma-custom-channel",
                "usage":{"input":100,"output":20,"cacheRead":200,"cacheWrite":30,
                    "reasoning":10,"totalTokens":350,"cost":{"total":cost}}}}),
        ]
    }
    fn read(paths: &[PathBuf]) -> Vec<UnifiedMessage> {
        reconcile_sources(
            paths
                .iter()
                .map(|p| (p.clone(), parse_proma_file(p)))
                .collect(),
        )
    }
    fn index(root: &Path, value: Value) {
        std::fs::write(root.join("agent-sessions.json"), value.to_string()).unwrap();
    }
    #[test]
    fn sdk_uses_persisted_time_and_nonoverlapping_buckets() {
        let temp = tempfile::tempdir().unwrap();
        let mut row = sdk("ui", Some("msg"), "session", 20);
        row["cwd"] = json!("/repo");
        row["message"]["service_tier"] = json!("priority");
        let path = write(
            temp.path(),
            "agent-sessions/ui.jsonl",
            vec![
                row,
                json!({"type":"result","usage":{"input_tokens":10000}}),
                json!({"type":"user","message":{"usage":{"input_tokens":9000}}}),
            ],
        );
        let messages = read(&[path]);
        assert_eq!(messages.len(), 1);
        let m = &messages[0];
        assert_eq!(m.tokens.total(), 350);
        assert_eq!(m.tokens.cache_write_1h, 20);
        assert_eq!(m.timestamp, 1778000000123);
        assert_eq!(m.session_id, "session");
        assert_eq!(m.workspace_key.as_deref(), Some("/repo"));
        assert_eq!(m.service_tier.as_deref(), Some("priority"));
        assert_ne!(m.cost_source, CostSource::ProviderReported);
    }
    #[test]
    fn pi_preserves_custom_model_reported_zero_or_paid_cost_and_fork_identity() {
        let temp = tempfile::tempdir().unwrap();
        for cost in [Some(0.0), Some(1.25), None, Some(-1.0)] {
            let a = write(
                temp.path(),
                "sdk-config/sessions/a.jsonl",
                pi_records("parent", "entry", cost),
            );
            let b = write(
                temp.path(),
                "sdk-config/sessions/b.jsonl",
                pi_records("fork", "entry", cost),
            );
            let messages = read(&[a, b]);
            assert_eq!(messages.len(), 1);
            let m = &messages[0];
            assert_eq!(m.tokens.total(), 350);
            assert_eq!(m.tokens.reasoning, 0);
            assert_eq!(m.model_id, "mimo-v2.5-pro");
            if let Some(cost) = cost.filter(|c| *c >= 0.0) {
                assert_eq!(m.cost, cost);
                assert_eq!(m.cost_source, CostSource::ProviderReported);
            } else {
                assert_ne!(m.cost_source, CostSource::ProviderReported);
            }
        }
    }
    #[test]
    fn native_pi_wins_over_unbound_display_mirror_by_sdk_session() {
        let temp = tempfile::tempdir().unwrap();
        let pi = write(
            temp.path(),
            "sdk-config/sessions/a.jsonl",
            pi_records("native", "entry", Some(0.2)),
        );
        let ui = write(
            temp.path(),
            "agent-sessions/ui.jsonl",
            vec![sdk("uuid", None, "native", 20)],
        );
        let messages = read(&[pi, ui]);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].cost, 0.2);
    }
    #[test]
    fn index_bindings_remove_fork_mirrors_but_preserve_missing_native_tail_and_legacy_history() {
        let temp = tempfile::tempdir().unwrap();
        let pi = write(
            temp.path(),
            "sdk-config/sessions/fork.jsonl",
            pi_records("fork", "entry", Some(0.2)),
        );
        let ui = write(
            temp.path(),
            "agent-sessions/ui.jsonl",
            vec![
                sdk("copy", None, "deleted-parent", 20),
                sdk("tail", None, "fork", 3),
                sdk("legacy", Some("old-claude"), "claude-session", 4),
            ],
        );
        index(
            temp.path(),
            json!({"version":2,"sessions":[{"id":"ui","piSessionFile":pi,
            "piEntryBindings":{"copy":"entry","tail":"missing-entry"}}]}),
        );
        let messages = read(&[pi.clone(), ui.clone()]);
        assert_eq!(messages.len(), 3);
        assert_eq!(
            messages
                .iter()
                .filter(|m| m.cost_source == CostSource::ProviderReported)
                .count(),
            1
        );
        std::fs::remove_file(pi).unwrap();
        assert_eq!(
            read(&[ui]).len(),
            3,
            "display records remain when SDK file disappears"
        );
    }
    #[test]
    fn relocated_index_uses_discovered_sdk_id_without_opening_recorded_paths() {
        let temp = tempfile::tempdir().unwrap();
        let pi = write(
            temp.path(),
            "sdk-config/sessions/fork.jsonl",
            pi_records("fork", "entry", None),
        );
        let ui = write(
            temp.path(),
            "agent-sessions/ui.jsonl",
            vec![sdk("copy", None, "parent", 20)],
        );
        index(
            temp.path(),
            json!({"sessions":[{"id":"ui","sdkSessionId":"fork",
            "piSessionFile":"/unavailable/original/root/private.jsonl","piEntryBindings":{"copy":"entry"}}]}),
        );
        assert_eq!(read(&[pi, ui]).len(), 1);
    }
    #[test]
    fn claude_native_display_and_fork_snapshots_count_once_without_collapsing_equal_calls() {
        let temp = tempfile::tempdir().unwrap();
        let native = write(
            temp.path(),
            "sdk-config/projects/repo/native.jsonl",
            vec![sdk("uuid", Some("msg"), "native", 20)],
        );
        let mut prefix = sdk("uuid", None, "native", 2);
        prefix["message"]["usage"]["cache_creation_input_tokens"] = json!(0);
        let ui = write(
            temp.path(),
            "agent-sessions/ui.jsonl",
            vec![
                prefix,
                sdk("other-uuid", Some("msg"), "native", 20),
                sdk("second", Some("different-call"), "native", 20),
            ],
        );
        let fork = write(
            temp.path(),
            "agent-sessions/fork.jsonl",
            vec![sdk("uuid", Some("msg"), "fork", 20)],
        );
        let messages = read(&[native, ui, fork]);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages.iter().map(|m| m.tokens.total()).sum::<i64>(), 700);
    }
    #[test]
    fn malformed_lines_negative_counters_and_unknown_models_do_not_hide_valid_usage() {
        let temp = tempfile::tempdir().unwrap();
        let mut row = sdk("", None, "native", 20);
        row["message"]["model"] = Value::Null;
        row["message"]["usage"]["input_tokens"] = json!(-100);
        let path = write(
            temp.path(),
            "agent-sessions/ui.jsonl",
            vec![row.clone(), row],
        );
        let content = std::fs::read(&path).unwrap();
        std::fs::write(
            &path,
            [
                b"\xef\xbb\xbf{broken}\r\n".as_slice(),
                &content,
                b"\n{truncated",
            ]
            .concat(),
        )
        .unwrap();
        let messages = read(&[path]);
        assert_eq!(
            messages.len(),
            2,
            "anonymous equal calls are not duplicates"
        );
        assert_eq!(messages[0].model_id, "unknown");
        assert_eq!(messages[0].tokens.input, 0);
    }
    #[test]
    fn billed_pi_operations_survive_display_precedence() {
        let temp = tempfile::tempdir().unwrap();
        let mut records = pi_records("native", "entry", Some(0.2));
        records.push(json!({"type":"compaction","id":"compaction-call","timestamp":"2026-05-05T12:02:00Z",
            "model":"mimo-v2.5-pro","provider":"proma-custom-channel","usage":{"input":10,"output":5,"cost":{"total":0.1}}}));
        let pi = write(temp.path(), "sdk-config/sessions/native.jsonl", records);
        let ui = write(
            temp.path(),
            "agent-sessions/ui.jsonl",
            vec![sdk("uuid", None, "native", 20)],
        );
        let messages = read(&[pi, ui]);
        assert_eq!(messages.len(), 2);
        assert!((messages.iter().map(|m| m.cost).sum::<f64>() - 0.3).abs() < 1e-10);
    }
    #[test]
    fn discovery_limits_usage_to_agent_and_sdk_stores() {
        let temp = tempfile::tempdir().unwrap();
        for relative in [
            ".proma/agent-sessions/ui.jsonl",
            ".proma/sdk-config/sessions/repo/native.jsonl",
            ".proma/sdk-config/projects/repo/legacy.jsonl",
            ".proma-dev/agent-sessions/dev.jsonl",
        ] {
            write(temp.path(), relative, vec![sdk("ui", None, "s", 1)]);
        }
        for relative in [
            ".proma/conversations/chat.jsonl",
            ".proma/sdk-config/credentials.jsonl",
            ".proma/workspaces/repo/secret.jsonl",
            ".proma/workspaces/agent-sessions/unrelated.jsonl",
            ".proma/workspaces/sdk-config/sessions/unrelated.jsonl",
            ".proma/agent-sessions/nested/unrelated.jsonl",
        ] {
            write(temp.path(), relative, vec![sdk("ui", None, "s", 1)]);
        }
        let scan = crate::scanner::scan_all_clients_with_env_strategy(
            temp.path().to_str().unwrap(),
            &["proma".into()],
            false,
        );
        assert_eq!(scan.get(crate::ClientId::Proma).len(), 4);
    }
    #[test]
    fn explicit_export_roots_are_supported_and_overlaps_are_deduplicated() {
        let home = tempfile::tempdir().unwrap();
        let export = home.path().join("exported-profile");
        let path = write(
            &export,
            "agent-sessions/ui.jsonl",
            vec![sdk("ui", None, "session", 1)],
        );
        let settings = serde_json::from_value(serde_json::json!({"extraScanPaths": {
            "proma": [export, path.parent().unwrap(), path.parent().unwrap()]
        }}))
        .unwrap();
        let scan = crate::scanner::scan_all_clients_with_scanner_settings(
            home.path().to_str().unwrap(),
            &["proma".into()],
            false,
            &settings,
        );
        assert_eq!(scan.get(crate::ClientId::Proma), &[path]);
    }

    #[test]
    #[cfg(unix)]
    fn discovery_does_not_follow_transcript_file_symlinks() {
        let temp = tempfile::tempdir().unwrap();
        let target = write(
            temp.path(),
            "unrelated.jsonl",
            vec![sdk("ui", None, "session", 1)],
        );
        let root = temp.path().join(".proma/agent-sessions");
        std::fs::create_dir_all(&root).unwrap();
        std::os::unix::fs::symlink(target, root.join("linked.jsonl")).unwrap();
        let scan = crate::scanner::scan_all_clients_with_env_strategy(
            temp.path().to_str().unwrap(),
            &["proma".into()],
            false,
        );
        assert!(scan.get(crate::ClientId::Proma).is_empty());
    }

    #[test]
    fn corrupted_or_oversized_index_keeps_independent_display_usage() {
        let temp = tempfile::tempdir().unwrap();
        let ui = write(
            temp.path(),
            "agent-sessions/ui.jsonl",
            vec![sdk("uuid", None, "s", 20)],
        );
        std::fs::write(temp.path().join("agent-sessions.json"), "{truncated").unwrap();
        assert_eq!(read(std::slice::from_ref(&ui)).len(), 1);
        std::fs::File::create(temp.path().join("agent-sessions.json"))
            .unwrap()
            .set_len(16 * 1024 * 1024 + 1)
            .unwrap();
        assert_eq!(read(&[ui]).len(), 1);
    }
}
