use std::fs;
use tempfile::TempDir;
use tokscale_core::sessions::codebuff::parse_codebuff_file;

fn write_chat(dir: &TempDir, channel: &str, project: &str, chat_id: &str, body: &str) -> std::path::PathBuf {
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
