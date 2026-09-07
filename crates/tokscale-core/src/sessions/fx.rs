//! fx (vercel-labs/fx) session parser
//!
//! Parses per-session usage snapshots from
//! `~/.fx/sessions/<sessionId>/usage-v2.json`, paired with the sibling
//! `session.json` (workspace root + authoritative timestamps) and the shared
//! `~/.fx/sessions/index.json` (human-readable session title).
//!
//! fx aggregates per-request token usage into one snapshot per session, so this
//! parser emits one `UnifiedMessage` per (session × model) entry — the same
//! session-level shape as other aggregate integrations (Kilo, Goose, Mux).
//! A session whose snapshot carries only the top-level aggregates (empty
//! `models`) is attributed to a synthetic `fx-unknown` model instead of being
//! dropped, so the session totals are never silently lost.
//!
//! The global `~/.fx/usage.jsonl` stream also exists (one `generation` record
//! per request) but carries no session id or workspace, so it is intentionally
//! not scanned here.
//!
//! Cache split. The two sidecars are read back differently because they sit at
//! different scopes. `session.json` is per-session, so it is a related file of
//! the snapshot's cache fingerprint (`fx_session_meta_path`) and a
//! sidecar-only edit invalidates exactly that one entry. `index.json` is one
//! file shared by every session, so it is deliberately *not* in any
//! fingerprint — appending a new session to it would otherwise invalidate every
//! other session's entry. Its title is resolved after the cache instead, by
//! `apply_session_titles`, which reads the index once per scan and writes the
//! title onto freshly parsed and cache-served messages alike. Titles are
//! collected per sessions directory, because an fx session id is only unique
//! within one `sessions/` tree.
//!
//! Cost provenance. `total_cost` is optional on the wire at both levels, so its
//! presence is preserved rather than defaulted: a snapshot that never carried a
//! cost must not be submitted as an authoritative $0.00. See `reported_cost`.

use super::utils::{file_modified_timestamp_ms, read_file_or_none};
use super::{normalize_workspace_key, workspace_label_from_key, CostSource, UnifiedMessage};
use crate::{provider_identity, TokenBreakdown};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const CLIENT_ID: &str = "fx";
// `fx-unknown` follows the `<client>-unknown` convention (trae.rs) for
// session usage whose per-model breakdown is unavailable: the session totals
// are still attributed instead of disappearing into an empty Model cell.
const UNKNOWN_MODEL: &str = "fx-unknown";

#[derive(Debug, Deserialize)]
struct FxUsageFile {
    #[allow(dead_code)]
    schema_version: Option<u32>,
    #[allow(dead_code)]
    session_id: Option<String>,
    #[serde(default)]
    snapshot: Option<FxSnapshot>,
}

#[derive(Debug, Default, Deserialize)]
struct FxSnapshot {
    #[allow(dead_code)]
    schema_version: Option<u32>,
    // Top-level session aggregates (session_usage.zig `Snapshot`) always
    // accompany `models`; the per-model entries are the breakdown of these
    // totals. They back the synthetic fallback below.
    //
    // `Option` so an omitted field and an explicit `null` stay distinguishable
    // from a recorded `0`; see [`reported_cost`].
    #[serde(default)]
    total_cost: Option<f64>,
    #[serde(default)]
    input_tokens: i64,
    #[serde(default)]
    output_tokens: i64,
    #[serde(default)]
    cache_read_tokens: i64,
    #[serde(default)]
    cache_write_tokens: i64,
    #[serde(default)]
    reasoning_tokens: Option<i64>,
    #[serde(default)]
    request_count: Option<i64>,
    #[serde(default)]
    models: Vec<FxModelUsage>,
}

#[derive(Debug, Default, Deserialize)]
struct FxModelUsage {
    #[serde(default)]
    model: Option<String>,
    /// `Option` so an omitted field and an explicit `null` stay distinguishable
    /// from a recorded `0`; see [`reported_cost`].
    #[serde(default)]
    total_cost: Option<f64>,
    #[serde(default)]
    input_tokens: i64,
    #[serde(default)]
    output_tokens: i64,
    #[serde(default)]
    cache_read_tokens: i64,
    #[serde(default)]
    cache_write_tokens: i64,
    // Nullable in the wire schema (`writeOptionalU64`), so these must be
    // `Option`; an explicit JSON `null` would otherwise fail the whole file
    // parse and drop the session.
    #[serde(default)]
    reasoning_tokens: Option<i64>,
    #[serde(default)]
    request_count: Option<i64>,
}

#[derive(Debug, Default, Deserialize)]
struct FxSessionMeta {
    #[allow(dead_code)]
    id: Option<String>,
    #[serde(default)]
    workspace_root: Option<String>,
    #[serde(default)]
    created_at_ms: Option<i64>,
    #[serde(default)]
    updated_at_ms: Option<i64>,
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
    let bytes = read_file_or_none(path)?;
    serde_json::from_slice(&bytes).ok()
}

/// Reuse a recorded `total_cost` (USD) only when it is present, finite and
/// non-negative; otherwise report `0.0` with [`CostSource::Unknown`] so the
/// dispatch pricing guard prices the message or excludes it as unpriced.
///
/// Both `total_cost` fields are optional on the wire. Defaulting them to `0.0`
/// threw the presence away, and stamping [`CostSource::ProviderReported`] on
/// the result then submitted a cost the snapshot never carried as an
/// authoritative $0.00 — indistinguishable from a session fx really did bill
/// nothing for.
///
/// A negative total is treated as unreported rather than clamped into an
/// authoritative zero. `cost` still lands on `0.0`, exactly what the previous
/// `.max(0.0)` produced, but the provenance stays [`CostSource::Unknown`]:
/// a negative aggregate is a value fx cannot have meant as a bill, so it must
/// not lock the message out of repricing. Same rule as `gjc::embedded_cost` and
/// `cursor::parse_finite_cost`.
fn reported_cost(total: Option<f64>) -> (f64, CostSource) {
    match total {
        Some(total) if total.is_finite() && total >= 0.0 => (total, CostSource::ProviderReported),
        _ => (0.0, CostSource::Unknown),
    }
}

/// The sibling `session.json` a snapshot's parse reads for the workspace root
/// and the authoritative timestamps.
///
/// Always returns a path, including for a session that has no `session.json`
/// yet: the cache records the absence so a later creation still invalidates the
/// entry instead of freezing the mtime fallback timestamp forever.
pub(crate) fn fx_session_meta_path(usage_path: &Path) -> PathBuf {
    usage_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("session.json")
}

/// Read `<sessions>/index.json` (`{"sessions":[{"id","title",...}]}`) into
/// `id -> title`. A missing, unreadable or malformed index contributes nothing;
/// the Sessions tab then shows the session id.
fn collect_index_titles(sessions_dir: &Path, titles: &mut HashMap<String, String>) {
    let Some(value) = read_json::<serde_json::Value>(&sessions_dir.join("index.json")) else {
        return;
    };
    let Some(sessions) = value.get("sessions").and_then(|s| s.as_array()) else {
        return;
    };
    for entry in sessions {
        let Some(id) = entry.get("id").and_then(|id| id.as_str()) else {
            continue;
        };
        let title = entry
            .get("title")
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .trim();
        if title.is_empty() {
            continue;
        }
        titles.insert(id.to_string(), title.to_string());
    }
}

/// Attach human-readable session titles to an fx lane's messages.
///
/// Runs after the source-message cache, not inside [`parse_fx_file`], because
/// `index.json` is shared by every fx session: making it part of a per-session
/// cache fingerprint would let one new session invalidate every other session's
/// entry. Resolving it here keeps a title-only edit free — nothing is
/// invalidated, the index is parsed once per scan rather than once per session,
/// and a cache-served message still gets the current title.
///
/// The title is assigned, not filled in: a message restored from the cache
/// carries whatever title the index held when it was written, so an edited or
/// removed title has to be able to overwrite it back to `None`.
///
/// `sources` pairs each scanned `usage-v2.json` path with the messages that
/// path produced, freshly parsed or restored from the cache. That pairing is
/// the provenance the lookup needs: an fx session id is only unique inside one
/// `sessions/` tree, and a second configured scan root (`extra_scan_paths`)
/// holding the same id used to hand its title to the first root's session,
/// because the map was keyed by id alone. Titles are collected per sessions
/// directory — the usage path's grandparent — and each message is resolved
/// under the directory it actually came from.
///
/// Only fx messages are touched, so a group that also carries another client's
/// messages cannot have their titles cleared.
pub fn apply_session_titles(sources: &mut [(PathBuf, Vec<UnifiedMessage>)]) {
    let mut titles: HashMap<PathBuf, HashMap<String, String>> = HashMap::new();
    for (path, messages) in sources.iter() {
        if messages.is_empty() {
            continue;
        }
        let Some(sessions_dir) = path.parent().and_then(Path::parent) else {
            continue;
        };
        if titles.contains_key(sessions_dir) {
            continue;
        }
        let mut per_root: HashMap<String, String> = HashMap::new();
        collect_index_titles(sessions_dir, &mut per_root);
        titles.insert(sessions_dir.to_path_buf(), per_root);
    }

    for (path, messages) in sources.iter_mut() {
        let per_root = path
            .parent()
            .and_then(Path::parent)
            .and_then(|sessions_dir| titles.get(sessions_dir));
        for message in messages.iter_mut() {
            if message.client != CLIENT_ID {
                continue;
            }
            message.session_title = per_root
                .and_then(|per_root| per_root.get(&message.session_id))
                .cloned();
        }
    }
}

/// Split a provider-prefixed fx model id (`zai/glm-5.2`) into `(provider,
/// model)` without dropping either half. Models without a `/` prefix are kept
/// whole and the provider is inferred downstream.
fn split_model(raw: &str) -> (Option<String>, String) {
    match raw.split_once('/') {
        Some((provider, rest)) if !provider.is_empty() && !rest.is_empty() => {
            (Some(provider.to_string()), rest.to_string())
        }
        _ => (None, raw.to_string()),
    }
}

/// Parse one `usage-v2.json` file (a single fx session).
/// Returns one `UnifiedMessage` per model with non-zero recorded usage, or one
/// synthetic `fx-unknown` message from the top-level session aggregates when
/// no per-model entry produced a message.
pub fn parse_fx_file(path: &Path) -> Vec<UnifiedMessage> {
    let Some(bytes) = read_file_or_none(path) else {
        return Vec::new();
    };
    let file: FxUsageFile = match serde_json::from_slice(&bytes) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };
    let Some(snapshot) = file.snapshot else {
        return Vec::new();
    };

    let session_dir = path.parent();

    let session_id = file
        .session_id
        .filter(|s| !s.is_empty())
        .or_else(|| {
            session_dir
                .and_then(|d| d.file_name())
                .and_then(|n| n.to_str())
                .map(str::to_string)
        })
        .unwrap_or_default();

    // Sibling `session.json` carries the workspace root and authoritative
    // timestamps; fall back to the usage file's mtime when absent.
    let meta: Option<FxSessionMeta> = read_json(&fx_session_meta_path(path));
    let workspace_root = meta.as_ref().and_then(|m| m.workspace_root.clone());
    let timestamp_ms = meta
        .as_ref()
        .and_then(|m| m.updated_at_ms.or(m.created_at_ms));
    let fallback_timestamp = file_modified_timestamp_ms(path);
    let timestamp = timestamp_ms.unwrap_or(fallback_timestamp);

    let workspace_key = workspace_root.as_deref().and_then(normalize_workspace_key);
    let workspace_label = workspace_key.as_deref().and_then(workspace_label_from_key);

    let mut messages = Vec::new();
    for model_usage in &snapshot.models {
        let raw_model = model_usage.model.as_deref().unwrap_or(UNKNOWN_MODEL);
        let tokens = TokenBreakdown {
            input: model_usage.input_tokens.max(0),
            output: model_usage.output_tokens.max(0),
            cache_read: model_usage.cache_read_tokens.max(0),
            cache_write: model_usage.cache_write_tokens.max(0),
            reasoning: model_usage.reasoning_tokens.unwrap_or(0).max(0),
        };
        if tokens.total() == 0 {
            continue;
        }

        let (provider, model_id) = split_model(raw_model);
        let provider_id = provider
            .as_deref()
            .and_then(provider_identity::canonical_provider)
            .or_else(|| {
                provider_identity::inferred_provider_from_model(&model_id).map(str::to_string)
            })
            .unwrap_or_else(|| "zai".to_string());

        let dedup_key = format!("fx:{session_id}:{model_id}");
        let (cost, cost_source) = reported_cost(model_usage.total_cost);

        messages.push(UnifiedMessage {
            client: CLIENT_ID.to_string(),
            model_id,
            provider_id,
            session_id: session_id.clone(),
            workspace_key: workspace_key.clone(),
            workspace_label: workspace_label.clone(),
            timestamp,
            date: String::new(),
            tokens,
            cost,
            cost_source,
            duration_ms: None,
            message_count: model_usage.request_count.unwrap_or(0).max(0) as i32,
            agent: None,
            dedup_key: Some(dedup_key),
            // Resolved after the cache by `apply_session_titles`; see the
            // module docs for why the shared index stays out of the parse.
            session_title: None,
            is_turn_start: false,
            model_attribution_conflicted: false,
        });
    }

    // fx also writes session-level aggregates at the top of the snapshot,
    // alongside the per-model breakdown. When no per-model entry produced a
    // message (empty `models`, or every entry with zero tokens), attribute the
    // session totals to a synthetic unknown model instead of dropping the
    // session's usage silently.
    if messages.is_empty() {
        let tokens = TokenBreakdown {
            input: snapshot.input_tokens.max(0),
            output: snapshot.output_tokens.max(0),
            cache_read: snapshot.cache_read_tokens.max(0),
            cache_write: snapshot.cache_write_tokens.max(0),
            reasoning: snapshot.reasoning_tokens.unwrap_or(0).max(0),
        };
        let request_count = snapshot.request_count.unwrap_or(0).max(0);
        let (cost, cost_source) = reported_cost(snapshot.total_cost);
        if tokens.total() > 0 || request_count > 0 || cost > 0.0 {
            messages.push(UnifiedMessage {
                client: CLIENT_ID.to_string(),
                model_id: UNKNOWN_MODEL.to_string(),
                provider_id: "zai".to_string(),
                session_id: session_id.clone(),
                workspace_key: workspace_key.clone(),
                workspace_label: workspace_label.clone(),
                timestamp,
                date: String::new(),
                tokens,
                cost,
                cost_source,
                duration_ms: None,
                message_count: request_count as i32,
                agent: None,
                dedup_key: Some(format!("fx:{session_id}:{UNKNOWN_MODEL}")),
                session_title: None,
                is_turn_start: false,
                model_attribution_conflicted: false,
            });
        }
    }
    messages
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TokenBreakdown;

    fn write_file(dir: &std::path::Path, name: &str, content: &str) -> std::path::PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, content).unwrap();
        path
    }

    fn fixture() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let sessions = dir.path().join("sessions");
        let session = sessions.join("sess-123");
        std::fs::create_dir_all(&session).unwrap();
        (dir, session)
    }

    /// Resolve titles for messages that all came from one `usage-v2.json`,
    /// the shape the lane builds when a scan turns up a single source.
    fn apply_titles_from(usage: &std::path::Path, messages: &mut Vec<UnifiedMessage>) {
        let mut sources = vec![(usage.to_path_buf(), std::mem::take(messages))];
        apply_session_titles(&mut sources);
        *messages = sources.remove(0).1;
    }

    #[test]
    fn parses_provider_prefixed_model_into_one_message() {
        let (dir, session) = fixture();
        write_file(
            &session,
            "session.json",
            r#"{"workspace_root":"/Users/alice/repo","updated_at_ms":1787196905040}"#,
        );
        write_file(
            &dir.path().join("sessions"),
            "index.json",
            r#"{"schema_version":3,"sessions":[{"id":"sess-123","workspace_root":"/Users/alice/repo","title":"Setup CI"}]}"#,
        );
        let usage = write_file(
            &session,
            "usage-v2.json",
            r#"{
              "schema_version":1,
              "session_id":"sess-123",
              "snapshot":{
                "schema_version":2,
                "total_cost":0.01,
                "request_count":2,
                "models":[{"model":"zai/glm-5.2","total_cost":0.01,"input_tokens":1539,"output_tokens":441,"cache_read_tokens":1069,"cache_write_tokens":7,"reasoning_tokens":3,"request_count":2}]
              }
            }"#,
        );

        let mut messages = parse_fx_file(&usage);
        // The parse leaves the title unset; the lane resolves it from the
        // shared index once per scan, after the cache.
        assert_eq!(messages[0].session_title, None);
        apply_titles_from(&usage, &mut messages);
        assert_eq!(messages.len(), 1);
        let msg = &messages[0];
        assert_eq!(msg.client, "fx");
        assert_eq!(msg.session_id, "sess-123");
        assert_eq!(msg.model_id, "glm-5.2");
        assert_eq!(msg.provider_id, "zai");
        assert_eq!(msg.workspace_key.as_deref(), Some("/Users/alice/repo"));
        assert_eq!(msg.session_title.as_deref(), Some("Setup CI"));
        assert_eq!(msg.timestamp, 1787196905040);
        assert_eq!(
            msg.tokens,
            TokenBreakdown {
                input: 1539,
                output: 441,
                cache_read: 1069,
                cache_write: 7,
                reasoning: 3,
            }
        );
        assert!((msg.cost - 0.01).abs() < 1e-9);
        assert_eq!(msg.message_count, 2);
        assert_eq!(msg.cost_source, CostSource::ProviderReported);
    }

    #[test]
    fn skips_session_with_no_usage() {
        let (_dir, session) = fixture();
        let usage = write_file(
            &session,
            "usage-v2.json",
            r#"{"schema_version":1,"session_id":"empty","snapshot":{"models":[],"request_count":0,"total_cost":0}}"#,
        );
        assert!(parse_fx_file(&usage).is_empty());
    }

    #[test]
    fn falls_back_to_synthetic_unknown_model_from_top_level_aggregates() {
        let (dir, session) = fixture();
        write_file(
            &session,
            "session.json",
            r#"{"workspace_root":"/Users/alice/repo","updated_at_ms":1787196905040}"#,
        );
        write_file(
            &dir.path().join("sessions"),
            "index.json",
            r#"{"schema_version":3,"sessions":[{"id":"sess-456","workspace_root":"/Users/alice/repo","title":"Refactor CLI"}]}"#,
        );
        let usage = write_file(
            &session,
            "usage-v2.json",
            r#"{
              "schema_version":1,
              "session_id":"sess-456",
              "snapshot":{
                "schema_version":2,
                "total_cost":0.014,
                "input_tokens":2000,
                "output_tokens":800,
                "cache_read_tokens":500,
                "cache_write_tokens":10,
                "reasoning_tokens":120,
                "request_count":3,
                "models":[]
              }
            }"#,
        );

        let mut messages = parse_fx_file(&usage);
        apply_titles_from(&usage, &mut messages);
        assert_eq!(messages.len(), 1);
        let msg = &messages[0];
        assert_eq!(msg.client, "fx");
        assert_eq!(msg.session_id, "sess-456");
        assert_eq!(msg.model_id, "fx-unknown");
        assert_eq!(msg.provider_id, "zai");
        assert_eq!(msg.workspace_key.as_deref(), Some("/Users/alice/repo"));
        assert_eq!(msg.session_title.as_deref(), Some("Refactor CLI"));
        assert_eq!(msg.timestamp, 1787196905040);
        assert_eq!(
            msg.tokens,
            TokenBreakdown {
                input: 2000,
                output: 800,
                cache_read: 500,
                cache_write: 10,
                reasoning: 120,
            }
        );
        assert!((msg.cost - 0.014).abs() < 1e-9);
        assert_eq!(msg.message_count, 3);
        assert_eq!(msg.cost_source, CostSource::ProviderReported);
        assert_eq!(msg.dedup_key.as_deref(), Some("fx:sess-456:fx-unknown"));
    }

    #[test]
    fn top_level_aggregates_do_not_double_count_when_models_present() {
        let (_dir, session) = fixture();
        let usage = write_file(
            &session,
            "usage-v2.json",
            r#"{
              "schema_version":1,
              "session_id":"s",
              "snapshot":{
                "total_cost":0.02,
                "input_tokens":3000,
                "output_tokens":1000,
                "request_count":4,
                "models":[{"model":"zai/glm-5.2","total_cost":0.02,"input_tokens":3000,"output_tokens":1000,"request_count":4}]
              }
            }"#,
        );
        let messages = parse_fx_file(&usage);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "glm-5.2");
    }

    #[test]
    fn tolerates_null_reasoning_and_request_count_fields() {
        // The wire schema serializes absent `reasoning_tokens`/`request_count`
        // as JSON `null`. An explicit `null` must not fail the file parse and
        // drop the session.
        let (_dir, session) = fixture();
        let usage = write_file(
            &session,
            "usage-v2.json",
            r#"{"schema_version":1,"session_id":"s3","snapshot":{"models":[{"model":"anthropic/claude-sonnet-4","total_cost":0.001,"input_tokens":10,"output_tokens":5,"reasoning_tokens":null,"request_count":null}]}}"#,
        );
        let messages = parse_fx_file(&usage);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.reasoning, 0);
        assert_eq!(messages[0].message_count, 0);
    }

    #[test]
    fn skips_model_entry_with_zero_tokens() {
        let (_dir, session) = fixture();
        let usage = write_file(
            &session,
            "usage-v2.json",
            r#"{"schema_version":1,"session_id":"s","snapshot":{"models":[{"model":"zai/glm-5.2","input_tokens":0,"output_tokens":0}]}}"#,
        );
        assert!(parse_fx_file(&usage).is_empty());
    }

    #[test]
    fn tolerates_missing_sibling_metadata() {
        let (_dir, session) = fixture();
        let usage = write_file(
            &session,
            "usage-v2.json",
            r#"{"schema_version":1,"session_id":"s2","snapshot":{"models":[{"model":"glm-5.2","input_tokens":10,"output_tokens":5}]}}"#,
        );
        let mut messages = parse_fx_file(&usage);
        apply_titles_from(&usage, &mut messages);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].provider_id, "zai");
        assert_eq!(messages[0].workspace_key, None);
        assert_eq!(messages[0].session_title, None);
    }

    #[test]
    fn apply_session_titles_overwrites_a_cache_served_title() {
        // A message restored from the source-message cache carries the title
        // the index held when the entry was written. A later rename must be
        // able to replace it, and a removed title must be able to clear it --
        // hence assignment rather than fill-if-empty.
        let (dir, session) = fixture();
        let sessions = dir.path().join("sessions");
        write_file(
            &sessions,
            "index.json",
            r#"{"schema_version":3,"sessions":[{"id":"sess-123","title":"Renamed later"}]}"#,
        );
        let usage = write_file(
            &session,
            "usage-v2.json",
            r#"{"schema_version":1,"session_id":"sess-123","snapshot":{"models":[{"model":"zai/glm-5.2","input_tokens":10,"output_tokens":5}]}}"#,
        );

        let mut messages = parse_fx_file(&usage);
        messages[0].session_title = Some("Stale cached title".to_string());
        apply_titles_from(&usage, &mut messages);
        assert_eq!(messages[0].session_title.as_deref(), Some("Renamed later"));

        write_file(
            &sessions,
            "index.json",
            r#"{"schema_version":3,"sessions":[{"id":"sess-123","title":"   "}]}"#,
        );
        apply_titles_from(&usage, &mut messages);
        assert_eq!(
            messages[0].session_title, None,
            "a blank title must clear the cached one, not leave it standing"
        );
    }

    #[test]
    fn apply_session_titles_leaves_other_clients_alone() {
        let (dir, session) = fixture();
        write_file(
            &dir.path().join("sessions"),
            "index.json",
            r#"{"schema_version":3,"sessions":[{"id":"sess-123","title":"Setup CI"}]}"#,
        );
        let usage = write_file(
            &session,
            "usage-v2.json",
            r#"{"schema_version":1,"session_id":"sess-123","snapshot":{"models":[{"model":"zai/glm-5.2","input_tokens":10,"output_tokens":5}]}}"#,
        );

        let mut messages = parse_fx_file(&usage);
        let mut foreign = messages[0].clone();
        foreign.client = "codex".to_string();
        foreign.session_title = Some("Someone else's title".to_string());
        messages.push(foreign);

        apply_titles_from(&usage, &mut messages);
        assert_eq!(messages[0].session_title.as_deref(), Some("Setup CI"));
        assert_eq!(
            messages[1].session_title.as_deref(),
            Some("Someone else's title")
        );
    }

    #[test]
    fn apply_session_titles_covers_every_session_under_one_index() {
        let dir = tempfile::tempdir().unwrap();
        let sessions = dir.path().join("sessions");
        let mut sources = Vec::new();
        for id in ["sess-a", "sess-b"] {
            let session = sessions.join(id);
            std::fs::create_dir_all(&session).unwrap();
            let usage = write_file(
                &session,
                "usage-v2.json",
                &format!(
                    r#"{{"schema_version":1,"session_id":"{id}","snapshot":{{"models":[{{"model":"zai/glm-5.2","input_tokens":10,"output_tokens":5}}]}}}}"#
                ),
            );
            sources.push((usage.clone(), parse_fx_file(&usage)));
        }
        write_file(
            &sessions,
            "index.json",
            r#"{"schema_version":3,"sessions":[{"id":"sess-a","title":"First"},{"id":"sess-b","title":"Second"}]}"#,
        );

        apply_session_titles(&mut sources);
        let mut titles: Vec<Option<&str>> = sources
            .iter()
            .flat_map(|(_, messages)| messages.iter())
            .map(|m| m.session_title.as_deref())
            .collect();
        titles.sort();
        assert_eq!(titles, vec![Some("First"), Some("Second")]);
    }

    /// Parse a one-model snapshot whose model entry carries `cost_field`
    /// verbatim (`""` for an omitted field) and return the single message.
    fn model_cost_message(cost_field: &str) -> UnifiedMessage {
        let (_dir, session) = fixture();
        let usage = write_file(
            &session,
            "usage-v2.json",
            &format!(
                r#"{{"schema_version":1,"session_id":"sess-123","snapshot":{{"models":[{{"model":"zai/glm-5.2",{cost_field}"input_tokens":10,"output_tokens":5}}]}}}}"#
            ),
        );
        let mut messages = parse_fx_file(&usage);
        assert_eq!(messages.len(), 1, "cost field {cost_field:?}");
        messages.remove(0)
    }

    /// Same, for the snapshot-level aggregate fallback: `models` is empty, so
    /// the synthetic `fx-unknown` message carries the top-level totals.
    fn snapshot_cost_message(cost_field: &str) -> UnifiedMessage {
        let (_dir, session) = fixture();
        let usage = write_file(
            &session,
            "usage-v2.json",
            &format!(
                r#"{{"schema_version":1,"session_id":"sess-123","snapshot":{{{cost_field}"input_tokens":10,"output_tokens":5,"request_count":1,"models":[]}}}}"#
            ),
        );
        let mut messages = parse_fx_file(&usage);
        assert_eq!(messages.len(), 1, "cost field {cost_field:?}");
        messages.remove(0)
    }

    /// Every way a snapshot can fail to state a cost, spelled the way fx
    /// writes it: the field omitted, an explicit `null`, and a negative
    /// aggregate. A non-finite value has no JSON spelling that reaches the
    /// parser — serde_json rejects an out-of-range literal like `1e400` and
    /// the whole file is dropped before any cost is read — so that branch is
    /// pinned on [`reported_cost`] directly in
    /// `a_non_finite_cost_is_never_provider_reported`.
    const UNREPORTED_COST_FIELDS: [&str; 3] =
        ["", r#""total_cost":null,"#, r#""total_cost":-0.5,"#];

    #[test]
    fn a_non_finite_cost_is_never_provider_reported() {
        for total in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert_eq!(
                reported_cost(Some(total)),
                (0.0, CostSource::Unknown),
                "{total} is not a cost fx can have meant"
            );
        }
        assert_eq!(reported_cost(None), (0.0, CostSource::Unknown));
        assert_eq!(reported_cost(Some(-0.5)), (0.0, CostSource::Unknown));
        assert_eq!(
            reported_cost(Some(0.0)),
            (0.0, CostSource::ProviderReported)
        );
        assert_eq!(
            reported_cost(Some(0.25)),
            (0.25, CostSource::ProviderReported)
        );
    }

    #[test]
    fn an_unreported_model_cost_is_not_submitted_as_an_authoritative_zero() {
        // `total_cost` is optional at both levels. Defaulting it to 0.0 and
        // stamping ProviderReported made "the snapshot did not say" look
        // exactly like "fx billed nothing", which locks the message out of
        // repricing and submits real usage at $0.00.
        for cost_field in UNREPORTED_COST_FIELDS {
            let msg = model_cost_message(cost_field);
            assert_eq!(msg.cost, 0.0, "cost field {cost_field:?}");
            assert_eq!(
                msg.cost_source,
                CostSource::Unknown,
                "cost field {cost_field:?} must stay repriceable"
            );
        }
    }

    #[test]
    fn an_unreported_snapshot_cost_is_not_submitted_as_an_authoritative_zero() {
        for cost_field in UNREPORTED_COST_FIELDS {
            let msg = snapshot_cost_message(cost_field);
            assert_eq!(msg.model_id, UNKNOWN_MODEL);
            assert_eq!(msg.cost, 0.0, "cost field {cost_field:?}");
            assert_eq!(
                msg.cost_source,
                CostSource::Unknown,
                "cost field {cost_field:?} must stay repriceable"
            );
        }
    }

    #[test]
    fn a_recorded_zero_cost_stays_provider_reported() {
        // The other half of the rule: a snapshot that really did state `0` is
        // authoritative and must not be repriced.
        let msg = model_cost_message(r#""total_cost":0,"#);
        assert_eq!(msg.cost, 0.0);
        assert_eq!(msg.cost_source, CostSource::ProviderReported);

        let msg = model_cost_message(r#""total_cost":0.004,"#);
        assert!((msg.cost - 0.004).abs() < 1e-9);
        assert_eq!(msg.cost_source, CostSource::ProviderReported);

        let msg = snapshot_cost_message(r#""total_cost":0,"#);
        assert_eq!(msg.cost, 0.0);
        assert_eq!(msg.cost_source, CostSource::ProviderReported);

        let msg = snapshot_cost_message(r#""total_cost":0.004,"#);
        assert!((msg.cost - 0.004).abs() < 1e-9);
        assert_eq!(msg.cost_source, CostSource::ProviderReported);
    }

    #[test]
    fn two_roots_sharing_a_session_id_keep_their_own_titles() {
        // fx session ids are only unique within one `sessions/` tree, and
        // `extra_scan_paths` lets a second tree be scanned in the same lane.
        // Keyed by id alone, whichever index was read last won for both.
        let dir = tempfile::tempdir().unwrap();
        let mut sources = Vec::new();
        for (root, title) in [("root-a", "Ship the parser"), ("root-b", "Fix the cache")] {
            let sessions = dir.path().join(root).join("sessions");
            let session = sessions.join("sess-shared");
            std::fs::create_dir_all(&session).unwrap();
            write_file(
                &sessions,
                "index.json",
                &format!(
                    r#"{{"schema_version":3,"sessions":[{{"id":"sess-shared","title":"{title}"}}]}}"#
                ),
            );
            let usage = write_file(
                &session,
                "usage-v2.json",
                r#"{"schema_version":1,"session_id":"sess-shared","snapshot":{"models":[{"model":"zai/glm-5.2","input_tokens":10,"output_tokens":5}]}}"#,
            );
            sources.push((usage.clone(), parse_fx_file(&usage)));
        }

        apply_session_titles(&mut sources);

        assert_eq!(
            sources[0].1[0].session_title.as_deref(),
            Some("Ship the parser")
        );
        assert_eq!(
            sources[1].1[0].session_title.as_deref(),
            Some("Fix the cache")
        );
        assert_eq!(sources[0].1[0].session_id, sources[1].1[0].session_id);
    }

    #[test]
    fn fx_session_meta_path_points_at_the_sibling_metadata() {
        let (_dir, session) = fixture();
        let usage = session.join("usage-v2.json");
        assert_eq!(fx_session_meta_path(&usage), session.join("session.json"));
    }
}
