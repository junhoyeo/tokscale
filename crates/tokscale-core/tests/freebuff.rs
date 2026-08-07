use std::fs;
use tempfile::TempDir;
use tokscale_core::sessions::freebuff::parse_freebuff_file;

/// Write a Freebuff `chat-messages.json` under `<base>/projects/<project>/chats/<id>/`
/// and a channel-root `settings.json` carrying the configured `freebuffModel`.
fn write_chat(
    base: &std::path::Path,
    project: &str,
    chat_id: &str,
    body: &str,
    freebuff_model: &str,
) -> std::path::PathBuf {
    let chat_dir = base
        .join("projects")
        .join(project)
        .join("chats")
        .join(chat_id);
    fs::create_dir_all(&chat_dir).unwrap();
    let msgs_path = chat_dir.join("chat-messages.json");
    fs::write(&msgs_path, body).unwrap();
    // Channel-root settings.json (Freebuff mirrors Codebuff's manicode layout).
    fs::write(
        base.join("settings.json"),
        format!("{{\"freebuffModel\": \"{freebuff_model}\"}}"),
    )
    .unwrap();
    msgs_path
}

#[test]
fn test_parse_freebuff_emits_estimated_tokens_per_turn() {
    let dir = TempDir::new().unwrap();
    let base = dir.path().join("manicode");
    let path = write_chat(
        &base,
        "my-project",
        "2026-08-07T05-20-31.453Z",
        r#"[
            { "variant": "user", "content": "hello world", "timestamp": "2026-08-07T05:20:31.453Z" },
            { "variant": "ai", "content": "", "blocks": [ { "type": "text", "content": "Hello!" } ], "timestamp": "2026-08-07T05:20:31.453Z" },
            { "variant": "user", "content": "thanks", "timestamp": "2026-08-07T05:20:31.453Z" },
            { "variant": "ai", "content": "", "blocks": [ { "type": "text", "content": "You're welcome" } ], "timestamp": "2026-08-07T05:20:31.453Z" }
        ]"#,
        "deepseek/deepseek-v4-flash",
    );

    let msgs = parse_freebuff_file(&path);
    assert_eq!(
        msgs.len(),
        2,
        "only assistant turns with content are emitted"
    );

    let first = &msgs[0];
    assert_eq!(first.client, "freebuff");
    assert_eq!(first.model_id, "deepseek/deepseek-v4-flash");
    assert!(first.provider_id.eq_ignore_ascii_case("deepseek"));
    assert!(first
        .session_id
        .ends_with("/my-project/2026-08-07T05-20-31.453Z"));
    // input from the prior user turn: "hello world" = 11 chars / 4 -> 3
    assert_eq!(first.tokens.input, 3);
    // output from this assistant's text: "Hello!" = 6 chars / 4 -> 2
    assert_eq!(first.tokens.output, 2);
    assert_eq!(first.tokens.cache_read, 0);
    assert_eq!(first.tokens.cache_write, 0);
    assert_eq!(first.message_count, 1);
    assert!(first.is_turn_start);

    let second = &msgs[1];
    // input from the second user turn: "thanks" = 6 chars / 4 -> 2
    assert_eq!(second.tokens.input, 2);
    // output: "You're welcome" = 13 chars / 4 -> 4
    assert_eq!(second.tokens.output, 4);
    assert!(second.is_turn_start);
}

#[test]
fn test_parse_freebuff_defers_codebuff_chats_with_authoritative_usage() {
    const CODEBUFF_CHAT: &str = r#"[
            { "variant": "user", "content": "hi", "timestamp": "2026-08-07T05:20:31.453Z" },
            { "variant": "ai",
              "timestamp": "2026-08-07T05:21:00.000Z",
              "metadata": {
                "model": "claude-sonnet-4-20250514",
                "usage": { "inputTokens": 500, "outputTokens": 200 }
              }
            }
        ]"#;
    let dir = TempDir::new().unwrap();
    let base = dir.path().join("manicode");
    let path = write_chat(
        &base,
        "proj",
        "2026-08-07T06-00-00.000Z",
        CODEBUFF_CHAT,
        "deepseek/deepseek-v4-flash",
    );

    // A real Codebuff chat (authoritative usage present) must be left to the
    // codebuff parser, never estimated as freebuff.
    assert!(parse_freebuff_file(&path).is_empty());
}

#[test]
fn test_parse_freebuff_skips_messages_without_text() {
    let dir = TempDir::new().unwrap();
    let base = dir.path().join("manicode");
    let path = write_chat(
        &base,
        "proj",
        "2026-08-07T07-00-00.000Z",
        r#"[
            { "variant": "user", "content": "hi", "timestamp": "2026-08-07T07:00:00.000Z" },
            { "variant": "ai", "content": "", "blocks": [ { "type": "mode-divider", "mode": "LITE" } ], "timestamp": "2026-08-07T07:00:00.000Z" }
        ]"#,
        "deepseek/deepseek-v4-flash",
    );

    // The mode-divider assistant message has no text, so nothing is estimated.
    assert!(parse_freebuff_file(&path).is_empty());
}
