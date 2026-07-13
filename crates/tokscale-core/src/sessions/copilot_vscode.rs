use super::{normalize_workspace_key, workspace_label_from_key, UnifiedMessage};
use crate::provider_identity::inferred_provider_from_model;
use crate::TokenBreakdown;
use serde_json::Value;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

pub fn parse_copilot_vscode_sessions(paths: &[PathBuf]) -> Vec<UnifiedMessage> {
    paths.iter().flat_map(|path| parse_file(path)).collect()
}

fn parse_file(path: &Path) -> Vec<UnifiedMessage> {
    let session_id = match path.file_stem().and_then(|s| s.to_str()) {
        Some(stem) => stem.to_string(),
        None => return Vec::new(),
    };

    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };

    let workspace = read_workspace_for_file(path);

    let mut requests: Vec<Value> = Vec::new();

    for line in BufReader::new(file).lines().map_while(Result::ok) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(obj) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };
        let kind = obj.get("kind").and_then(Value::as_i64).unwrap_or(-1);
        match kind {
            0 => {
                if let Some(arr) = obj.pointer("/v/requests").and_then(Value::as_array) {
                    requests.extend(arr.iter().cloned());
                }
            }
            2 => {
                if let Some(k) = obj.get("k").and_then(Value::as_array) {
                    let is_requests = k
                        .first()
                        .and_then(Value::as_str)
                        .map(|s| s == "requests")
                        .unwrap_or(false);
                    if is_requests {
                        if let Some(arr) = obj.get("v").and_then(Value::as_array) {
                            requests.extend(arr.iter().cloned());
                        }
                    }
                }
            }
            _ => {}
        }
    }

    requests
        .iter()
        .filter_map(|req| request_to_message(req, &session_id, &workspace))
        .collect()
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
    let hash_dir = jsonl_path.parent()?.parent()?;
    let workspace_json = hash_dir.join("workspace.json");

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
}
