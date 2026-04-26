use std::fs;
use tempfile::TempDir;
use tokscale_core::sessions::codebuff::parse_codebuff_file;

fn write_chat(
    dir: &TempDir,
    channel: &str,
    project: &str,
    chat_id: &str,
    body: &str,
) -> std::path::PathBuf {
    let chat_dir = dir
        .path()
        .join(channel)
        .join("projects")
        .join(project)
        .join("chats")
        .join(chat_id);
    fs::create_dir_all(&chat_dir).unwrap();
    let msgs_path = chat_dir.join("chat-messages.json");
    fs::write(&msgs_path, body).unwrap();
    msgs_path
}

#[test]
fn test_parse_codebuff_emits_one_event_per_assistant_message_with_usage() {
    let dir = TempDir::new().unwrap();
    let path = write_chat(
        &dir,
        "manicode",
        "my-project",
        "2025-12-20T12-00-00.000Z",
        r#"[
            { "variant": "user", "content": "hello", "timestamp": "2025-12-20T12:00:00.000Z" },
            {
                "variant": "ai",
                "timestamp": "2025-12-20T12:00:05.000Z",
                "metadata": {
                    "model": "claude-sonnet-4-20250514",
                    "usage": {
                        "inputTokens": 500,
                        "outputTokens": 200,
                        "cacheCreationInputTokens": 300,
                        "cacheReadInputTokens": 100
                    }
                },
                "credits": 1.25
            },
            {
                "variant": "user",
                "content": "thanks",
                "timestamp": "2025-12-20T12:00:10.000Z"
            },
            {
                "variant": "ai",
                "timestamp": "2025-12-20T12:00:15.000Z",
                "metadata": {
                    "model": "openai/gpt-5",
                    "codebuff": {
                        "usage": {
                            "prompt_tokens": 750,
                            "completion_tokens": 80,
                            "prompt_tokens_details": { "cached_tokens": 100 }
                        }
                    }
                }
            }
        ]"#,
    );

    let msgs = parse_codebuff_file(&path);
    assert_eq!(msgs.len(), 2);

    let first = &msgs[0];
    assert_eq!(first.client, "codebuff");
    assert_eq!(first.model_id, "claude-sonnet-4-20250514");
    assert_eq!(first.provider_id, "anthropic");
    assert_eq!(first.tokens.input, 500);
    assert_eq!(first.tokens.output, 200);
    assert_eq!(first.tokens.cache_write, 300);
    assert_eq!(first.tokens.cache_read, 100);
    assert_eq!(first.cost, 1.25);
    assert!(first
        .session_id
        .ends_with("/my-project/2025-12-20T12-00-00.000Z"));

    let second = &msgs[1];
    assert_eq!(second.model_id, "openai/gpt-5");
    assert_eq!(second.provider_id, "openai");
    assert_eq!(second.tokens.input, 750);
    assert_eq!(second.tokens.output, 80);
    assert_eq!(second.tokens.cache_read, 100);
}

#[test]
fn test_parse_codebuff_recovers_usage_from_run_state_history_when_metadata_is_empty() {
    let dir = TempDir::new().unwrap();
    let path = write_chat(
        &dir,
        "manicode-dev",
        "sandbox",
        "2025-12-22T09-30-00.000Z",
        r#"[
            { "variant": "user", "content": "run", "timestamp": "2025-12-22T09:30:00.000Z" },
            {
                "variant": "assistant",
                "timestamp": "2025-12-22T09:30:02.500Z",
                "metadata": {
                    "runState": {
                        "sessionState": {
                            "mainAgentState": {
                                "messageHistory": [
                                    { "role": "user", "providerOptions": {} },
                                    {
                                        "role": "assistant",
                                        "providerOptions": {
                                            "codebuff": {
                                                "model": "openrouter/anthropic/claude-opus-4-1",
                                                "usage": {
                                                    "inputTokens": 2000,
                                                    "outputTokens": 800,
                                                    "cacheReadInputTokens": 400
                                                }
                                            }
                                        }
                                    }
                                ]
                            }
                        }
                    }
                }
            }
        ]"#,
    );

    let msgs = parse_codebuff_file(&path);
    assert_eq!(msgs.len(), 1);
    let m = &msgs[0];
    assert_eq!(m.model_id, "openrouter/anthropic/claude-opus-4-1");
    assert_eq!(m.provider_id, "anthropic");
    assert_eq!(m.tokens.input, 2000);
    assert_eq!(m.tokens.output, 800);
    assert_eq!(m.tokens.cache_read, 400);
    assert!(m.session_id.starts_with("manicode-dev/sandbox/"));
}

#[test]
fn test_parse_codebuff_returns_empty_for_missing_or_non_array_file() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("chat-messages.json");
    fs::write(&path, r#"{"not":"an array"}"#).unwrap();
    assert!(parse_codebuff_file(&path).is_empty());

    let missing = dir.path().join("nope.json");
    assert!(parse_codebuff_file(&missing).is_empty());
}

#[test]
fn test_parse_codebuff_uses_chat_id_for_timestamp_when_message_has_none() {
    // Regression: before fixing the chat-id parser, a global `-`→`:` replace
    // corrupted the date portion and every message in a chat missing a
    // per-message timestamp silently fell back to the file mtime.
    let dir = TempDir::new().unwrap();
    let path = write_chat(
        &dir,
        "manicode",
        "proj",
        "2025-12-14T10-00-00.000Z",
        r#"[
            {
                "variant": "ai",
                "metadata": {
                    "model": "claude-sonnet-4-20250514",
                    "usage": { "inputTokens": 10, "outputTokens": 5 }
                }
            }
        ]"#,
    );

    let msgs = parse_codebuff_file(&path);
    assert_eq!(msgs.len(), 1);
    // 2025-12-14T10:00:00.000Z → epoch ms
    assert_eq!(msgs[0].timestamp, 1_765_706_400_000_i64);
}

#[test]
fn test_parse_codebuff_unknown_model_falls_back_to_unknown_provider() {
    let dir = TempDir::new().unwrap();
    let path = write_chat(
        &dir,
        "manicode",
        "proj",
        "2025-12-15T11-00-00.000Z",
        r#"[
            {
                "variant": "ai",
                "timestamp": "2025-12-15T11:00:01.000Z",
                "metadata": {
                    "usage": { "inputTokens": 100, "outputTokens": 50 }
                }
            }
        ]"#,
    );

    let msgs = parse_codebuff_file(&path);
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].model_id, "codebuff-unknown");
    assert_eq!(
        msgs[0].provider_id, "unknown",
        "unknown models must not be silently attributed to anthropic"
    );
}

#[test]
fn test_parse_codebuff_dedup_key_is_stable_for_same_history() {
    let dir = TempDir::new().unwrap();
    let body = r#"[
        {
            "variant": "ai",
            "id": "msg_abc123",
            "timestamp": "2025-12-16T12:00:00.000Z",
            "metadata": {
                "model": "claude-sonnet-4-20250514",
                "usage": { "inputTokens": 10, "outputTokens": 5 }
            }
        }
    ]"#;
    let path_a = write_chat(&dir, "manicode", "proj", "2025-12-16T12-00-00.000Z", body);
    let path_b = write_chat(
        &dir,
        "manicode-dev",
        "proj",
        "2025-12-16T12-00-00.000Z",
        body,
    );

    let msgs_a = parse_codebuff_file(&path_a);
    let msgs_b = parse_codebuff_file(&path_b);

    assert_eq!(msgs_a.len(), 1);
    assert_eq!(msgs_b.len(), 1);
    assert!(msgs_a[0].dedup_key.is_some());
    assert_eq!(
        msgs_a[0].dedup_key, msgs_b[0].dedup_key,
        "same upstream message id should yield identical dedup keys across channels"
    );
    assert_eq!(msgs_a[0].dedup_key.as_deref(), Some("msg_abc123"));
}

#[test]
fn test_parse_codebuff_dedup_key_falls_back_when_id_missing() {
    let dir = TempDir::new().unwrap();
    let path = write_chat(
        &dir,
        "manicode",
        "proj",
        "2025-12-17T13-00-00.000Z",
        r#"[
            {
                "variant": "ai",
                "timestamp": "2025-12-17T13:00:00.000Z",
                "metadata": {
                    "model": "claude-sonnet-4-20250514",
                    "usage": { "inputTokens": 10, "outputTokens": 5 }
                }
            }
        ]"#,
    );

    let msgs = parse_codebuff_file(&path);
    assert_eq!(msgs.len(), 1);
    let key = msgs[0].dedup_key.as_deref().expect("dedup_key required");
    assert!(
        key.starts_with("codebuff:"),
        "fallback dedup key should be namespaced; got {key}"
    );
}

#[test]
fn test_parse_codebuff_run_state_skips_entries_missing_provider_options() {
    // Regression: the run-state recovery walked the message history in
    // reverse, and the most recent assistant entry can lack providerOptions
    // while an earlier one carries the real usage payload. The recovery loop
    // must continue past entries without providerOptions instead of bailing
    // out of the entire function.
    let dir = TempDir::new().unwrap();
    let path = write_chat(
        &dir,
        "manicode",
        "proj",
        "2025-12-18T14-00-00.000Z",
        r#"[
            {
                "variant": "assistant",
                "timestamp": "2025-12-18T14:00:00.000Z",
                "metadata": {
                    "runState": {
                        "sessionState": {
                            "mainAgentState": {
                                "messageHistory": [
                                    {
                                        "role": "assistant",
                                        "providerOptions": {
                                            "codebuff": {
                                                "model": "claude-sonnet-4-20250514",
                                                "usage": { "inputTokens": 1234, "outputTokens": 56 }
                                            }
                                        }
                                    },
                                    { "role": "user" },
                                    { "role": "assistant" }
                                ]
                            }
                        }
                    }
                }
            }
        ]"#,
    );

    let msgs = parse_codebuff_file(&path);
    assert_eq!(
        msgs.len(),
        1,
        "earlier assistant entry's providerOptions must survive missing providerOptions on later entry"
    );
    assert_eq!(msgs[0].tokens.input, 1234);
    assert_eq!(msgs[0].tokens.output, 56);
    assert_eq!(msgs[0].model_id, "claude-sonnet-4-20250514");
}
