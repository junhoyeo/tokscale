use super::utils::lossy_lines;
use super::{normalize_workspace_key, workspace_label_from_key, UnifiedMessage};
use crate::provider_identity::inferred_provider_from_model;
use crate::TokenBreakdown;
use serde_json::Value;
use std::io::BufReader;
use std::path::{Path, PathBuf};

pub fn parse_copilot_vscode_sessions(paths: &[PathBuf]) -> Vec<UnifiedMessage> {
    let mut messages = Vec::new();
    parse_copilot_vscode_sessions_into(paths, &mut |_, message| messages.push(message));
    messages
}

/// Parse VS Code Copilot sessions into an incremental consumer.
///
/// The production submit path folds each message as it arrives. Keeping that
/// consumer here, at the parser boundary, avoids materialising a second vector
/// containing one `UnifiedMessage` per request after the request projection has
/// already accumulated the session. The collecting wrapper above remains for
/// callers that genuinely need ownership of the complete result. The path each
/// message came from is handed along because callers label messages per file.
pub(crate) fn parse_copilot_vscode_sessions_into(
    paths: &[PathBuf],
    on_message: &mut dyn FnMut(&Path, UnifiedMessage),
) {
    for group in session_groups(paths) {
        // VS Code copies a session into workspace storage when an empty window
        // enters a workspace, and keeps the empty-window original. Later writes
        // go only to the workspace copy, so a request held by both files has
        // its current state in the workspace copy and the preserved original is
        // a stale prefix. Accepting each key once, workspace copy first, keeps
        // the current state while still admitting requests that exist only in
        // the original (the copies diverge after the migration).
        let mut accepted: std::collections::HashSet<String> = std::collections::HashSet::new();
        for path in group {
            parse_file_into(path, &mut |message| {
                if let Some(key) = message.dedup_key.as_deref() {
                    if !accepted.insert(key.to_string()) {
                        return;
                    }
                }
                on_message(path, message);
            });
        }
    }
}

/// Group files that hold copies of one chat session, newest workspace copies
/// first and empty-window originals last.
///
/// VS Code identifies a session by its file name, and the copies it makes keep
/// that name, so the file stem is the session id across storage roots.
fn session_groups(paths: &[PathBuf]) -> Vec<Vec<&Path>> {
    let mut groups: Vec<Vec<&Path>> = Vec::new();
    let mut by_id: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();

    for path in paths {
        let Some(session_id) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        let index = *by_id.entry(session_id).or_insert_with(|| {
            groups.push(Vec::new());
            groups.len() - 1
        });
        groups[index].push(path.as_path());
    }

    // Later writes go to the new workspace copy, making it newer than the
    // preserved original. Equal mtimes keep the caller's order.
    for group in &mut groups {
        group.sort_by_key(|path| {
            (
                is_empty_window_session(path),
                std::cmp::Reverse(
                    path.metadata()
                        .and_then(|metadata| metadata.modified())
                        .unwrap_or(std::time::UNIX_EPOCH),
                ),
            )
        });
    }
    groups
}

/// Whether `path` sits in the store VS Code uses for windows with no folder or
/// workspace open. Sessions migrated out of such a window are copied into
/// workspace storage with the original left behind.
fn is_empty_window_session(path: &Path) -> bool {
    path.components()
        .any(|component| component.as_os_str() == "emptyWindowChatSessions")
}

fn parse_file_into(path: &Path, on_message: &mut dyn FnMut(UnifiedMessage)) {
    let session_id = match path.file_stem().and_then(|s| s.to_str()) {
        Some(stem) => stem.to_string(),
        None => return,
    };

    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return,
    };

    let workspace = read_workspace_for_file(path);

    let mut requests: Vec<Value> = Vec::new();

    for line in lossy_lines(BufReader::new(file)) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(mut obj) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };
        let kind = obj.get("kind").and_then(Value::as_i64).unwrap_or(-1);
        match kind {
            0 => {
                // Move the requests out of the line rather than cloning them.
                // `obj` is dropped at the end of this iteration, so a clone
                // holds the whole session DOM twice at its peak -- on a large
                // agent session that doubling is measured in gigabytes.
                if let Some(slot) = obj.pointer_mut("/v/requests") {
                    if let Value::Array(arr) = slot.take() {
                        requests.extend(arr.into_iter().map(|mut req| {
                            prune_request(&mut req);
                            req
                        }));
                    }
                }
            }
            1 => {
                // The payload is taken out of the line before the path is read
                // so it can be moved into the request rather than cloned. A
                // streamed response body arrives through this arm, and `obj`
                // is dropped at the end of the iteration either way.
                let value = obj.get_mut("v").map(Value::take);
                if let (Some(value), Some(k)) = (value, obj.get("k").and_then(Value::as_array)) {
                    if k.first().and_then(Value::as_str) == Some("requests") {
                        if let Some(index) = k.get(1).and_then(|v| v.as_u64()).map(|u| u as usize) {
                            // Dropping out-of-range updates is intentional: padding placeholders would mint timestamp-0 messages.
                            if let Some(req) = requests.get_mut(index) {
                                apply_update(req, &k[2..], value);
                                // Pruning after the write rather than refusing
                                // the update keeps one code path for every
                                // shape an update can take, including a write
                                // that replaces `result` wholesale. The payload
                                // was moved in, not copied, so the discarded
                                // branches cost no extra peak.
                                prune_request(req);
                            }
                        }
                    }
                }
            }
            2 => {
                if let Some(k) = obj.get("k").and_then(Value::as_array) {
                    // Only top-level `["requests"]` appends grow the requests
                    // vec. Nested appends like `["requests", 0, "response"]`
                    // carry no token data, and letting them extend the vec
                    // would shift the positions that kind-1 updates address.
                    let is_requests =
                        k.len() == 1 && k.first().and_then(Value::as_str) == Some("requests");
                    if is_requests {
                        if let Some(Value::Array(arr)) = obj.get_mut("v").map(Value::take) {
                            requests.extend(arr.into_iter().map(|mut req| {
                                prune_request(&mut req);
                                req
                            }));
                        }
                    }
                }
            }
            _ => {}
        }
    }

    // Consume requests instead of borrowing the whole vector while building a
    // second corpus-sized result. Each projected JSON value is released as
    // soon as its message has been folded by the caller.
    for request in requests {
        if let Some(message) = request_to_message(&request, &session_id, &workspace) {
            on_message(message);
        }
    }
}

/// Drop everything from a request that [`request_to_message`] does not read.
///
/// A session file is not one big snapshot: the `kind:0` line is a near-empty
/// stub (1,340 bytes against a 1.3 MB file on the local corpus) and the content
/// arrives as `kind:1`/`kind:2` lines appended over the session's life. So peak
/// memory is not set by parsing any single line -- it is set by the `requests`
/// vec accumulating full request bodies across all of them.
///
/// Almost none of that body is ever looked at. Across 60 local session files,
/// requests held 1,077,254 bytes, of which 45,500 are reachable from the reads
/// below; `variableData`, `edits`, `response`, and `metadata.toolCallResults`
/// alone are 87%. Pruning on the way in keeps the accumulation proportional to
/// what the parse actually needs.
///
/// **This function and `request_to_message` must agree.** A field the reader
/// starts reading without being kept here reads as absent, which silently drops
/// usage rather than failing -- the worst shape of bug this file can have.
/// `projection_keeps_every_field_request_to_message_reads` fails if they
/// diverge.
fn prune_request(req: &mut Value) {
    let Some(request) = req.as_object_mut() else {
        return;
    };
    request.retain(|key, _| {
        matches!(
            key.as_str(),
            "promptTokens" | "completionTokens" | "timestamp" | "modelId" | "result"
        )
    });

    let Some(result) = request.get_mut("result").and_then(Value::as_object_mut) else {
        return;
    };
    result.retain(|key, _| key == "metadata");

    let Some(metadata) = result.get_mut("metadata").and_then(Value::as_object_mut) else {
        return;
    };
    metadata.retain(|key, _| {
        matches!(
            key.as_str(),
            "promptTokens" | "outputTokens" | "resolvedModel" | "toolCallRounds"
        )
    });

    // Only `thinking.tokens` is summed out of a round, and the rounds carry the
    // tool arguments and results that make them the largest surviving field.
    let Some(rounds) = metadata
        .get_mut("toolCallRounds")
        .and_then(Value::as_array_mut)
    else {
        return;
    };
    for round in rounds {
        let Some(round) = round.as_object_mut() else {
            continue;
        };
        round.retain(|key, _| key == "thinking");
        if let Some(thinking) = round.get_mut("thinking").and_then(Value::as_object_mut) {
            thinking.retain(|key, _| key == "tokens");
        }
    }
}

/// Upper bound for numeric indexes in an update path. A corrupted line with a
/// huge index would otherwise drive the padding loops below into an unbounded
/// allocation.
const MAX_PATH_ARRAY_INDEX: usize = 4096;

fn apply_update(target: &mut Value, path: &[Value], value: Value) {
    // The last path segment is split off so the terminal write can move the
    // payload instead of cloning it. Only one write ever happens per call, but
    // when that write lives inside the descent loop the payload has to be
    // cloned to satisfy the borrow checker -- and the payload here is a whole
    // response-metadata blob.
    let Some((last, parents)) = path.split_last() else {
        *target = value;
        return;
    };

    let mut current = target;
    for key in parents {
        if let Some(k_str) = key.as_str() {
            if !current.is_object() {
                *current = serde_json::Value::Object(serde_json::Map::new());
            }
            let obj = current.as_object_mut().unwrap();
            if !obj.contains_key(k_str) {
                obj.insert(
                    k_str.to_string(),
                    serde_json::Value::Object(serde_json::Map::new()),
                );
            }
            current = obj.get_mut(k_str).unwrap();
        } else if let Some(k_idx) = key.as_u64() {
            if k_idx > MAX_PATH_ARRAY_INDEX as u64 {
                return;
            }
            let idx = k_idx as usize;
            if !current.is_array() {
                *current = serde_json::Value::Array(Vec::new());
            }
            let arr = current.as_array_mut().unwrap();
            while arr.len() <= idx {
                arr.push(serde_json::Value::Null);
            }
            current = &mut arr[idx];
        } else {
            return;
        }
    }

    if let Some(k_str) = last.as_str() {
        if !current.is_object() {
            *current = serde_json::Value::Object(serde_json::Map::new());
        }
        current
            .as_object_mut()
            .unwrap()
            .insert(k_str.to_string(), value);
    } else if let Some(k_idx) = last.as_u64() {
        if k_idx > MAX_PATH_ARRAY_INDEX as u64 {
            return;
        }
        let idx = k_idx as usize;
        if !current.is_array() {
            *current = serde_json::Value::Array(Vec::new());
        }
        let arr = current.as_array_mut().unwrap();
        if idx < arr.len() {
            arr[idx] = value;
        } else {
            while arr.len() < idx {
                arr.push(serde_json::Value::Null);
            }
            arr.push(value);
        }
    }
}

fn request_to_message(
    req: &Value,
    session_id: &str,
    workspace: &Option<(String, Option<String>)>,
) -> Option<UnifiedMessage> {
    let prompt_tokens = req
        .get("promptTokens")
        .and_then(Value::as_i64)
        .or_else(|| {
            req.pointer("/result/metadata/promptTokens")
                .and_then(Value::as_i64)
        })
        .unwrap_or(0);

    let completion_tokens = req
        .get("completionTokens")
        .and_then(Value::as_i64)
        .or_else(|| {
            req.pointer("/result/metadata/outputTokens")
                .and_then(Value::as_i64)
        })
        .unwrap_or(0);

    if prompt_tokens == 0 && completion_tokens == 0 {
        return None;
    }

    let timestamp_ms = req.get("timestamp").and_then(Value::as_i64).unwrap_or(0);

    let resolved_model = req
        .pointer("/result/metadata/resolvedModel")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty());

    let model_id_raw = req
        .get("modelId")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty());

    let model_id = resolved_model
        .or_else(|| model_id_raw.map(|m| m.strip_prefix("copilot/").unwrap_or(m)))
        .unwrap_or("auto")
        .to_string();

    // Filter: only include requests that are copilot-originated
    // (modelId starts with "copilot/" or resolved model is present)
    let is_copilot = resolved_model.is_some()
        || model_id_raw
            .map(|m| m.starts_with("copilot/"))
            .unwrap_or(false);
    if !is_copilot {
        return None;
    }

    let provider_id = inferred_provider_from_model(&model_id)
        .unwrap_or("github-copilot")
        .to_string();

    let reasoning_tokens: i64 = req
        .pointer("/result/metadata/toolCallRounds")
        .and_then(Value::as_array)
        .map(|rounds| {
            rounds
                .iter()
                .filter_map(|r| r.pointer("/thinking/tokens").and_then(Value::as_i64))
                .sum()
        })
        .unwrap_or(0);

    let tokens = TokenBreakdown {
        input: prompt_tokens.max(0),
        output: completion_tokens.max(0),
        cache_read: 0,
        cache_write: 0,
        cache_write_1h: 0,
        reasoning: reasoning_tokens.max(0),
    };

    let dedup_key = format!("copilot-vscode:{}:{}", session_id, timestamp_ms);

    let mut message = UnifiedMessage::new_with_dedup(
        "copilot",
        model_id,
        provider_id,
        session_id.to_string(),
        timestamp_ms,
        tokens,
        0.0,
        Some(dedup_key),
    );

    if let Some((key, label)) = workspace {
        message.set_workspace(Some(key.clone()), label.clone());
    }

    Some(message)
}

fn read_workspace_for_file(jsonl_path: &Path) -> Option<(String, Option<String>)> {
    // Path: workspaceStorage/{hash}/chatSessions/{uuid}.jsonl
    // workspace.json is at: workspaceStorage/{hash}/workspace.json
    let sessions_dir = jsonl_path.parent()?;
    if sessions_dir.file_name().and_then(|name| name.to_str()) != Some("chatSessions") {
        return None;
    }
    let workspace_json = sessions_dir.parent()?.join("workspace.json");

    let contents = std::fs::read_to_string(&workspace_json).ok()?;
    let obj: Value = serde_json::from_str(&contents).ok()?;

    let folder = obj
        .get("folder")
        .and_then(Value::as_str)
        .or_else(|| obj.get("workspace").and_then(Value::as_str))?;

    // folder is a URI like "file:///Users/alice/project"
    let path_str = if let Some(stripped) = folder.strip_prefix("file://") {
        // On Windows "file:///C:/..." → strip "file://" leaving "/C:/..."
        // normalize_workspace_key handles slashes
        stripped
    } else {
        folder
    };

    let workspace_key = normalize_workspace_key(path_str)?;
    let workspace_label = workspace_label_from_key(&workspace_key);
    Some((workspace_key, workspace_label))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_jsonl(path: &Path, lines: &[&str]) {
        let mut f = std::fs::File::create(path).unwrap();
        for line in lines {
            writeln!(f, "{}", line).unwrap();
        }
    }

    /// The projection's allowlist and `request_to_message`'s reads are two
    /// spellings of one list, in two places. This pins them together: a
    /// request carrying every readable field must survive pruning with the
    /// identical message coming out the other side.
    ///
    /// A field added to the reader but not to `prune_request` reads as absent
    /// after pruning, so this fails rather than letting the usage disappear.
    #[test]
    fn projection_keeps_every_field_request_to_message_reads() {
        // Every field the reader touches, each with a value it could not
        // produce by accident, wrapped in the noise a real request carries.
        let full = serde_json::json!({
            "requestId": "r-full",
            "timestamp": 1_783_918_304_896i64,
            "modelId": "copilot/gpt-5.3",
            "promptTokens": 4242,
            "completionTokens": 777,
            "variableData": {"junk": "x".repeat(64)},
            "edits": ["noise", "noise"],
            "response": [{"value": "y".repeat(64)}],
            "result": {
                "timings": {"totalElapsed": 1234},
                "details": "noise",
                "metadata": {
                    "promptTokens": 5150,
                    "outputTokens": 888,
                    "resolvedModel": "gpt-5.3-codex",
                    "toolCallResults": {"big": "z".repeat(128)},
                    "renderedUserMessage": "w".repeat(128),
                    "toolCallRounds": [
                        {"thinking": {"tokens": 30, "text": "noise"}, "response": "noise"},
                        {"thinking": {"tokens": 70}, "toolCalls": ["noise"]},
                    ],
                },
            },
        });

        let mut pruned = full.clone();
        prune_request(&mut pruned);

        let workspace = None;
        let before = request_to_message(&full, "session-full", &workspace)
            .expect("the unpruned request is a copilot message");
        let after = request_to_message(&pruned, "session-full", &workspace)
            .expect("pruning must not make a readable request unreadable");

        assert_eq!(
            before, after,
            "prune_request dropped a field request_to_message reads"
        );
        // Guard against the test passing because nothing was read at all.
        assert_eq!(before.tokens.input, 4242);
        assert_eq!(before.tokens.output, 777);
        assert_eq!(before.tokens.reasoning, 100);
        assert_eq!(before.model_id, "gpt-5.3-codex");
    }

    #[test]
    fn projection_drops_the_bodies_it_never_reads() {
        let mut req = serde_json::json!({
            "timestamp": 1000,
            "modelId": "copilot/auto",
            "promptTokens": 10,
            "completionTokens": 5,
            "variableData": {"a": 1},
            "edits": [1, 2, 3],
            "response": ["body"],
            "result": {
                "timings": {"t": 1},
                "metadata": {
                    "resolvedModel": "gpt-5.3",
                    "toolCallResults": {"big": "x"},
                    "renderedUserMessage": "y",
                    "toolCallRounds": [{"thinking": {"tokens": 7}, "toolCalls": ["c"]}],
                },
            },
        });
        prune_request(&mut req);

        let obj = req.as_object().unwrap();
        for dropped in ["variableData", "edits", "response", "requestId"] {
            assert!(!obj.contains_key(dropped), "{dropped} should be pruned");
        }
        let metadata = req
            .pointer("/result/metadata")
            .unwrap()
            .as_object()
            .unwrap();
        for dropped in ["toolCallResults", "renderedUserMessage"] {
            assert!(
                !metadata.contains_key(dropped),
                "{dropped} should be pruned"
            );
        }
        assert!(req.pointer("/result/timings").is_none());
        // A round keeps its thinking tokens and nothing else.
        let round = req
            .pointer("/result/metadata/toolCallRounds/0")
            .unwrap()
            .as_object()
            .unwrap();
        assert!(!round.contains_key("toolCalls"));
        assert_eq!(
            req.pointer("/result/metadata/toolCallRounds/0/thinking/tokens")
                .and_then(Value::as_i64),
            Some(7)
        );
    }

    #[test]
    fn parse_kind0_with_requests() {
        let dir = tempfile::tempdir().unwrap();
        let sessions_dir = dir.path().join("chatSessions");
        std::fs::create_dir_all(&sessions_dir).unwrap();
        let uuid = "550e8400-e29b-41d4-a716-446655440000";
        let path = sessions_dir.join(format!("{}.jsonl", uuid));

        write_jsonl(
            &path,
            &[
                r#"{"kind":0,"v":{"requests":[{"requestId":"r1","timestamp":1783918304896,"modelId":"copilot/auto","completionTokens":154,"promptTokens":22079,"result":{"metadata":{"promptTokens":22079,"outputTokens":154,"resolvedModel":"gpt-5.3-codex"}}}]}}"#,
            ],
        );

        let messages = parse_copilot_vscode_sessions(&[path]);
        assert_eq!(messages.len(), 1);
        let m = &messages[0];
        assert_eq!(m.client, "copilot");
        assert_eq!(m.session_id, uuid);
        assert_eq!(m.model_id, "gpt-5.3-codex");
        assert_eq!(m.timestamp, 1783918304896);
        assert_eq!(m.tokens.input, 22079);
        assert_eq!(m.tokens.output, 154);
        assert_eq!(m.tokens.reasoning, 0);
        assert_eq!(
            m.dedup_key.as_deref(),
            Some(format!("copilot-vscode:{}:1783918304896", uuid).as_str())
        );
    }

    #[test]
    fn parses_empty_window_session_updates() {
        let dir = tempfile::tempdir().unwrap();
        // Empty-window layout: <sessionId>.jsonl directly under
        // globalStorage/emptyWindowChatSessions, outside any workspace hash.
        let global_storage = dir.path().join("globalStorage");
        let sessions_dir = global_storage.join("emptyWindowChatSessions");
        std::fs::create_dir_all(&sessions_dir).unwrap();
        let uuid = "950e8400-e29b-41d4-a716-446655440004";
        let path = sessions_dir.join(format!("{}.jsonl", uuid));
        std::fs::write(
            global_storage.join("workspace.json"),
            r#"{"folder":"file:///not/the/session/workspace"}"#,
        )
        .unwrap();

        // Shape seen in real empty-window sessions: a near-empty kind:0 stub,
        // the request appended by kind:2, and tokens arriving as kind:1
        // path updates with a BYOK modelId that only resolvedModel can
        // attribute to Copilot.
        write_jsonl(
            &path,
            &[
                r#"{"kind":0,"v":{"version":3,"requests":[]}}"#,
                r#"{"kind":2,"k":["requests"],"v":[{"requestId":"r0","timestamp":1790000000000,"modelId":"byok.example/byok.example:::custom-model-a"}]}"#,
                r#"{"kind":1,"k":["requests",0,"result"],"v":{"metadata":{"resolvedModel":"byok.example:::custom-model-a"}}}"#,
                r#"{"kind":1,"k":["requests",0,"promptTokens"],"v":1000}"#,
                r#"{"kind":1,"k":["requests",0,"completionTokens"],"v":200}"#,
            ],
        );

        let messages = parse_copilot_vscode_sessions(&[path]);
        assert_eq!(messages.len(), 1);
        let m = &messages[0];
        assert_eq!(m.session_id, uuid);
        assert_eq!(m.model_id, "byok.example:::custom-model-a");
        assert_eq!(m.tokens.input, 1000);
        assert_eq!(m.tokens.output, 200);
        // globalStorage/workspace.json is not per-workspace metadata, so the
        // message must stay unattributed rather than mis-attributed.
        assert!(m.workspace_key.is_none());
        assert!(m.workspace_label.is_none());
    }

    /// The two paths a migrated session can leave behind: the workspace copy
    /// later writes go to, and the original kept in the empty-window store.
    fn copied_session_paths(home: &Path, session_id: &str) -> (PathBuf, PathBuf) {
        let workspace_dir = home.join("workspaceStorage/2f1c9a/chatSessions");
        let empty_window_dir = home.join("globalStorage/emptyWindowChatSessions");
        std::fs::create_dir_all(&workspace_dir).unwrap();
        std::fs::create_dir_all(&empty_window_dir).unwrap();
        (
            workspace_dir.join(format!("{}.jsonl", session_id)),
            empty_window_dir.join(format!("{}.jsonl", session_id)),
        )
    }

    #[test]
    fn counts_a_request_stored_in_both_copies_once() {
        let dir = tempfile::tempdir().unwrap();
        let uuid = "960e8400-e29b-41d4-a716-446655440005";
        let (workspace_path, empty_window_path) = copied_session_paths(dir.path(), uuid);
        // The migration copies the file, so both copies start out identical.
        let lines = [
            r#"{"kind":0,"v":{"requests":[{"timestamp":1000,"modelId":"copilot/gpt-4o","promptTokens":1000}]}}"#,
        ];
        write_jsonl(&workspace_path, &lines);
        write_jsonl(&empty_window_path, &lines);

        let messages =
            parse_copilot_vscode_sessions(&[workspace_path.clone(), empty_window_path.clone()]);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.input, 1000);

        let mut streamed = Vec::new();
        parse_copilot_vscode_sessions_into(&[workspace_path, empty_window_path], &mut |_, m| {
            streamed.push(m.tokens.input);
        });
        assert_eq!(streamed, vec![1000]);
    }

    #[test]
    fn keeps_requests_that_only_one_copy_has() {
        let dir = tempfile::tempdir().unwrap();
        let uuid = "970e8400-e29b-41d4-a716-446655440006";
        let (workspace_path, empty_window_path) = copied_session_paths(dir.path(), uuid);
        // Shared request 1000, then each copy gains its own request after the
        // migration. Dropping either file wholesale would lose real usage.
        write_jsonl(
            &workspace_path,
            &[
                r#"{"kind":0,"v":{"requests":[{"timestamp":1000,"modelId":"copilot/gpt-4o","promptTokens":10}]}}"#,
                r#"{"kind":2,"k":["requests"],"v":[{"timestamp":2000,"modelId":"copilot/gpt-4o","promptTokens":20}]}"#,
            ],
        );
        write_jsonl(
            &empty_window_path,
            &[
                r#"{"kind":0,"v":{"requests":[{"timestamp":1000,"modelId":"copilot/gpt-4o","promptTokens":10}]}}"#,
                r#"{"kind":2,"k":["requests"],"v":[{"timestamp":3000,"modelId":"copilot/gpt-4o","promptTokens":30}]}"#,
            ],
        );

        let messages = parse_copilot_vscode_sessions(&[empty_window_path, workspace_path]);
        assert_eq!(
            messages
                .iter()
                .map(|m| (m.timestamp, m.tokens.input))
                .collect::<Vec<_>>(),
            vec![(1000, 10), (2000, 20), (3000, 30)]
        );
    }

    #[test]
    fn prefers_the_workspace_copy_when_a_copied_request_diverges() {
        let dir = tempfile::tempdir().unwrap();
        let uuid = "980e8400-e29b-41d4-a716-446655440007";
        let (workspace_path, empty_window_path) = copied_session_paths(dir.path(), uuid);
        // Same request, but the workspace copy carries the state written after
        // the migration. The empty-window original is a stale prefix, even when
        // it is handed to the parser first.
        write_jsonl(
            &workspace_path,
            &[
                r#"{"kind":0,"v":{"requests":[{"timestamp":1000,"modelId":"copilot/gpt-4o","promptTokens":1500}]}}"#,
            ],
        );
        write_jsonl(
            &empty_window_path,
            &[
                r#"{"kind":0,"v":{"requests":[{"timestamp":1000,"modelId":"copilot/gpt-4o","promptTokens":1000}]}}"#,
            ],
        );

        let messages = parse_copilot_vscode_sessions(&[empty_window_path, workspace_path]);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.input, 1500);
    }

    #[test]
    fn prefers_the_newer_workspace_copy_when_a_request_diverges() {
        let dir = tempfile::tempdir().unwrap();
        let uuid = "990e8400-e29b-41d4-a716-446655440009";
        let older_dir = dir.path().join("workspaceStorage/aaaaaa/chatSessions");
        let newer_dir = dir.path().join("workspaceStorage/zzzzzz/chatSessions");
        std::fs::create_dir_all(&older_dir).unwrap();
        std::fs::create_dir_all(&newer_dir).unwrap();
        let older_path = older_dir.join(format!("{}.jsonl", uuid));
        let newer_path = newer_dir.join(format!("{}.jsonl", uuid));
        write_jsonl(
            &older_path,
            &[
                r#"{"kind":0,"v":{"requests":[{"timestamp":1000,"modelId":"copilot/gpt-4o","promptTokens":1000}]}}"#,
            ],
        );
        write_jsonl(
            &newer_path,
            &[
                r#"{"kind":0,"v":{"requests":[{"timestamp":1000,"modelId":"copilot/gpt-4o","promptTokens":1500}]}}"#,
            ],
        );
        for (path, seconds) in [(&older_path, 100), (&newer_path, 200)] {
            std::fs::File::options()
                .write(true)
                .open(path)
                .unwrap()
                .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(seconds))
                .unwrap();
        }

        // Lexical path order would choose the stale copy; migration recency
        // must choose the workspace copy that receives later writes.
        let messages = parse_copilot_vscode_sessions(&[older_path, newer_path]);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.input, 1500);
    }

    #[test]
    fn streaming_parser_preserves_path_and_request_order() {
        let dir = tempfile::tempdir().unwrap();
        let sessions_dir = dir.path().join("chatSessions");
        std::fs::create_dir_all(&sessions_dir).unwrap();
        let first = sessions_dir.join("first.jsonl");
        let second = sessions_dir.join("second.jsonl");

        write_jsonl(
            &first,
            &[
                r#"{"kind":0,"v":{"requests":[{"timestamp":1000,"modelId":"copilot/gpt-4o","promptTokens":11},{"timestamp":2000,"modelId":"copilot/gpt-4o","promptTokens":22}]}}"#,
            ],
        );
        write_jsonl(
            &second,
            &[
                r#"{"kind":2,"k":["requests"],"v":[{"timestamp":3000,"modelId":"copilot/gpt-4o","promptTokens":33}]}"#,
            ],
        );

        let mut emitted = Vec::new();
        parse_copilot_vscode_sessions_into(&[first, second], &mut |_, message| {
            // Keep only a tiny projection: the streaming API must not require
            // its consumer to retain each full message.
            emitted.push((message.session_id, message.timestamp, message.tokens.input));
        });

        assert_eq!(
            emitted,
            vec![
                ("first".to_string(), 1000, 11),
                ("first".to_string(), 2000, 22),
                ("second".to_string(), 3000, 33),
            ]
        );
    }

    #[test]
    fn parse_kind2_array_append() {
        let dir = tempfile::tempdir().unwrap();
        let sessions_dir = dir.path().join("chatSessions");
        std::fs::create_dir_all(&sessions_dir).unwrap();
        let uuid = "650e8400-e29b-41d4-a716-446655440001";
        let path = sessions_dir.join(format!("{}.jsonl", uuid));

        write_jsonl(
            &path,
            &[
                r#"{"kind":0,"v":{"requests":[]}}"#,
                r#"{"kind":2,"k":["requests"],"v":[{"requestId":"r2","timestamp":1783918310000,"modelId":"copilot/auto","completionTokens":200,"promptTokens":5000,"result":{"metadata":{"promptTokens":5000,"outputTokens":200,"resolvedModel":"gpt-5.3-codex","toolCallRounds":[{"thinking":{"tokens":88}},{"thinking":{"tokens":12}}]}}}]}"#,
            ],
        );

        let messages = parse_copilot_vscode_sessions(&[path]);
        assert_eq!(messages.len(), 1);
        let m = &messages[0];
        assert_eq!(m.tokens.input, 5000);
        assert_eq!(m.tokens.output, 200);
        assert_eq!(m.tokens.reasoning, 100);
    }

    #[test]
    fn keeps_parsing_requests_after_an_undecodable_line() {
        let dir = tempfile::tempdir().unwrap();
        let sessions_dir = dir.path().join("chatSessions");
        std::fs::create_dir_all(&sessions_dir).unwrap();
        let path = sessions_dir.join("cccccccc-0000-0000-0000-000000000000.jsonl");

        let mut fixture = Vec::new();
        fixture.extend_from_slice(br#"{"kind":0,"v":{"requests":[]}}"#);
        fixture.push(b'\n');
        // A lone 0xff can never appear in valid UTF-8, so `BufRead::lines()`
        // reports this line as `InvalidData`.
        fixture.extend_from_slice(b"{\"kind\":9,\"v\":\"\xff\xfe\"}\n");
        fixture.extend_from_slice(
            br#"{"kind":2,"k":["requests"],"v":[{"requestId":"r9","timestamp":1783918310000,"modelId":"copilot/auto","completionTokens":200,"promptTokens":5000,"result":{"metadata":{"promptTokens":5000,"outputTokens":200,"resolvedModel":"gpt-5.3-codex"}}}]}"#,
        );
        fixture.push(b'\n');
        std::fs::write(&path, &fixture).unwrap();

        let messages = parse_copilot_vscode_sessions(&[path]);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.input, 5000);
    }

    #[test]
    fn skips_zero_token_requests() {
        let dir = tempfile::tempdir().unwrap();
        let sessions_dir = dir.path().join("chatSessions");
        std::fs::create_dir_all(&sessions_dir).unwrap();
        let path = sessions_dir.join("aaaaaaaa-0000-0000-0000-000000000000.jsonl");

        write_jsonl(
            &path,
            &[
                r#"{"kind":2,"k":["requests"],"v":[{"requestId":"r0","timestamp":1000,"modelId":"copilot/auto","completionTokens":0,"promptTokens":0}]}"#,
            ],
        );

        assert!(parse_copilot_vscode_sessions(&[path]).is_empty());
    }

    #[test]
    fn model_fallback_from_model_id() {
        let dir = tempfile::tempdir().unwrap();
        let sessions_dir = dir.path().join("chatSessions");
        std::fs::create_dir_all(&sessions_dir).unwrap();
        let path = sessions_dir.join("bbbbbbbb-0000-0000-0000-000000000000.jsonl");

        // No resolvedModel, only modelId with "copilot/" prefix
        write_jsonl(
            &path,
            &[
                r#"{"kind":2,"k":["requests"],"v":[{"requestId":"r3","timestamp":2000,"modelId":"copilot/gpt-4o","completionTokens":50,"promptTokens":300}]}"#,
            ],
        );

        let messages = parse_copilot_vscode_sessions(&[path]);
        assert_eq!(messages.len(), 1);
        // "copilot/" prefix stripped
        assert_eq!(messages[0].model_id, "gpt-4o");
    }

    #[test]
    fn reasoning_tokens_summed_from_tool_call_rounds() {
        let dir = tempfile::tempdir().unwrap();
        let sessions_dir = dir.path().join("chatSessions");
        std::fs::create_dir_all(&sessions_dir).unwrap();
        let path = sessions_dir.join("cccccccc-0000-0000-0000-000000000000.jsonl");

        write_jsonl(
            &path,
            &[
                r#"{"kind":2,"k":["requests"],"v":[{"requestId":"r4","timestamp":3000,"modelId":"copilot/auto","completionTokens":10,"promptTokens":100,"result":{"metadata":{"resolvedModel":"gpt-5.3-codex","toolCallRounds":[{"thinking":{"tokens":30}},{"thinking":{"tokens":70}}]}}}]}"#,
            ],
        );

        let messages = parse_copilot_vscode_sessions(&[path]);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.reasoning, 100);
    }

    #[test]
    fn non_copilot_model_id_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let sessions_dir = dir.path().join("chatSessions");
        std::fs::create_dir_all(&sessions_dir).unwrap();
        let path = sessions_dir.join("dddddddd-0000-0000-0000-000000000000.jsonl");

        // modelId does not start with "copilot/" and no resolvedModel
        write_jsonl(
            &path,
            &[
                r#"{"kind":2,"k":["requests"],"v":[{"requestId":"r5","timestamp":4000,"modelId":"some-other-extension/model","completionTokens":50,"promptTokens":300}]}"#,
            ],
        );

        assert!(parse_copilot_vscode_sessions(&[path]).is_empty());
    }
    #[test]
    fn parse_kind1_path_updates() {
        let dir = tempfile::tempdir().unwrap();
        let sessions_dir = dir.path().join("chatSessions");
        std::fs::create_dir_all(&sessions_dir).unwrap();
        let uuid = "750e8400-e29b-41d4-a716-446655440002";
        let path = sessions_dir.join(format!("{}.jsonl", uuid));

        write_jsonl(
            &path,
            &[
                r#"{"kind":0,"v":{"requests":[{"requestId":"r3","timestamp":1783918320000,"agent":{"id":"github.copilot"}}]}}"#,
                r#"{"kind":1,"k":["requests",0,"promptTokens"],"v":18561}"#,
                r#"{"kind":1,"k":["requests",0,"completionTokens"],"v":143}"#,
                r#"{"kind":1,"k":["requests",0,"result"],"v":{"metadata":{"promptTokens":18561,"outputTokens":143,"resolvedModel":"gpt-5.6-luna"}}}"#,
            ],
        );

        let messages = parse_copilot_vscode_sessions(&[path]);
        assert_eq!(messages.len(), 1);
        let m = &messages[0];
        assert_eq!(m.model_id, "gpt-5.6-luna");
        assert_eq!(m.tokens.input, 18561);
        assert_eq!(m.tokens.output, 143);
    }

    #[test]
    fn nested_kind2_appends_do_not_shift_kind1_request_indexes() {
        let dir = tempfile::tempdir().unwrap();
        let sessions_dir = dir.path().join("chatSessions");
        std::fs::create_dir_all(&sessions_dir).unwrap();
        let uuid = "850e8400-e29b-41d4-a716-446655440003";
        let path = sessions_dir.join(format!("{}.jsonl", uuid));

        // Mirrors a real multi-request session: placeholder requests arrive
        // without token data, kind-1 updates fill them in by position, and
        // response parts stream in as nested kind-2 appends in between. If
        // the nested append leaked into the top-level requests vec, the
        // kind-1 updates for index 1 would land on a response part instead
        // of r1, minting a timestamp-0 message with r1's tokens.
        write_jsonl(
            &path,
            &[
                r#"{"kind":0,"v":{"requests":[{"requestId":"r0","timestamp":1783918330000,"agent":{"id":"github.copilot"}}]}}"#,
                r#"{"kind":1,"k":["requests",0,"promptTokens"],"v":12000}"#,
                r#"{"kind":1,"k":["requests",0,"completionTokens"],"v":120}"#,
                r#"{"kind":1,"k":["requests",0,"result"],"v":{"metadata":{"promptTokens":12000,"outputTokens":120,"resolvedModel":"gpt-5.3-codex"}}}"#,
                r#"{"kind":2,"k":["requests",0,"response"],"v":[{"kind":"markdownContent","value":"part1"},{"kind":"markdownContent","value":"part2"},{"kind":"toolInvocationSerialized"}]}"#,
                r#"{"kind":2,"k":["requests"],"v":[{"requestId":"r1","timestamp":1783918340000,"agent":{"id":"github.copilot"}}]}"#,
                r#"{"kind":1,"k":["requests",1,"promptTokens"],"v":15000}"#,
                r#"{"kind":1,"k":["requests",1,"completionTokens"],"v":250}"#,
                r#"{"kind":1,"k":["requests",1,"result"],"v":{"metadata":{"promptTokens":15000,"outputTokens":250,"resolvedModel":"gpt-5.6-luna"}}}"#,
            ],
        );

        let messages = parse_copilot_vscode_sessions(&[path]);
        assert_eq!(messages.len(), 2);

        let m0 = &messages[0];
        assert_eq!(m0.timestamp, 1783918330000);
        assert_eq!(m0.model_id, "gpt-5.3-codex");
        assert_eq!(m0.tokens.input, 12000);
        assert_eq!(m0.tokens.output, 120);

        let m1 = &messages[1];
        assert_eq!(m1.timestamp, 1783918340000);
        assert_eq!(m1.model_id, "gpt-5.6-luna");
        assert_eq!(m1.tokens.input, 15000);
        assert_eq!(m1.tokens.output, 250);
    }

    fn path(segments: &[Value]) -> Vec<Value> {
        segments.to_vec()
    }

    #[test]
    fn apply_update_replaces_the_root_on_an_empty_path() {
        let mut target = serde_json::json!({"a": 1});
        apply_update(&mut target, &[], serde_json::json!("replaced"));
        assert_eq!(target, serde_json::json!("replaced"));
    }

    #[test]
    fn apply_update_creates_missing_object_segments() {
        let mut target = serde_json::json!({});
        apply_update(
            &mut target,
            &path(&[
                Value::from("result"),
                Value::from("metadata"),
                Value::from("outputTokens"),
            ]),
            serde_json::json!(143),
        );
        assert_eq!(
            target,
            serde_json::json!({"result": {"metadata": {"outputTokens": 143}}})
        );
    }

    #[test]
    fn apply_update_overwrites_a_non_object_segment_it_must_descend_through() {
        let mut target = serde_json::json!({"result": 7});
        apply_update(
            &mut target,
            &path(&[Value::from("result"), Value::from("metadata")]),
            serde_json::json!("x"),
        );
        assert_eq!(target, serde_json::json!({"result": {"metadata": "x"}}));
    }

    #[test]
    fn apply_update_pads_an_array_when_the_terminal_index_is_past_the_end() {
        let mut target = serde_json::json!({"response": ["a"]});
        apply_update(
            &mut target,
            &path(&[Value::from("response"), Value::from(3)]),
            serde_json::json!("d"),
        );
        assert_eq!(
            target,
            serde_json::json!({"response": ["a", null, null, "d"]})
        );
    }

    #[test]
    fn apply_update_overwrites_an_in_range_array_index() {
        let mut target = serde_json::json!({"response": ["a", "b", "c"]});
        apply_update(
            &mut target,
            &path(&[Value::from("response"), Value::from(1)]),
            serde_json::json!("B"),
        );
        assert_eq!(target, serde_json::json!({"response": ["a", "B", "c"]}));
    }

    #[test]
    fn apply_update_pads_an_array_it_descends_through() {
        let mut target = serde_json::json!({});
        apply_update(
            &mut target,
            &path(&[Value::from("response"), Value::from(2), Value::from("kind")]),
            serde_json::json!("markdownContent"),
        );
        assert_eq!(
            target,
            serde_json::json!({"response": [null, null, {"kind": "markdownContent"}]})
        );
    }

    #[test]
    fn apply_update_drops_an_oversized_terminal_index_instead_of_padding() {
        let mut target = serde_json::json!({"response": []});
        apply_update(
            &mut target,
            &path(&[
                Value::from("response"),
                Value::from(MAX_PATH_ARRAY_INDEX as u64 + 1),
            ]),
            serde_json::json!("boom"),
        );
        assert_eq!(target, serde_json::json!({"response": []}));
    }

    #[test]
    fn apply_update_drops_an_oversized_intermediate_index_instead_of_padding() {
        let mut target = serde_json::json!({"response": []});
        apply_update(
            &mut target,
            &path(&[
                Value::from("response"),
                Value::from(MAX_PATH_ARRAY_INDEX as u64 + 1),
                Value::from("kind"),
            ]),
            serde_json::json!("boom"),
        );
        assert_eq!(target, serde_json::json!({"response": []}));
    }

    #[test]
    fn apply_update_ignores_a_path_segment_that_is_neither_a_key_nor_an_index() {
        let mut target = serde_json::json!({"a": 1});
        apply_update(&mut target, &path(&[Value::Null]), serde_json::json!("x"));
        assert_eq!(target, serde_json::json!({"a": 1}));
    }
}
