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
            { "variant": "ai", "content": "", "blocks": [ { "type": "text", "content": "Hello!" } ], "timestamp": "2026-08-07T05:20:31.453Z",
              "metadata": { "runState": { "sessionState": { "mainAgentState": { "agentType": "base2-free-deepseek-flash" } } } } },
            { "variant": "user", "content": "thanks", "timestamp": "2026-08-07T05:20:31.453Z" },
            { "variant": "ai", "content": "", "blocks": [ { "type": "text", "content": "You're welcome" } ], "timestamp": "2026-08-07T05:20:31.453Z",
              "metadata": { "runState": { "sessionState": { "mainAgentState": { "agentType": "base2-free-deepseek-flash" } } } } }
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
    // output: "You're welcome" = 14 chars / 4 -> 4
    assert_eq!(second.tokens.output, 4);
    assert!(second.is_turn_start);
}

#[test]
fn test_parse_freebuff_defers_codebuff_chats_with_authoritative_usage() {
    // Freebuff-marked, yet carrying authoritative usage: the codebuff parser
    // owns any chat with real token counts, so this must not be estimated too.
    const CODEBUFF_CHAT: &str = r#"[
            { "variant": "user", "content": "hi", "timestamp": "2026-08-07T05:20:31.453Z" },
            { "variant": "ai",
              "timestamp": "2026-08-07T05:21:00.000Z",
              "blocks": [ { "type": "text", "content": "Hello!" } ],
              "metadata": {
                "model": "claude-sonnet-4-20250514",
                "usage": { "inputTokens": 500, "outputTokens": 200 },
                "runState": { "sessionState": { "mainAgentState": { "agentType": "base2-free" } } }
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
            { "variant": "ai", "content": "", "blocks": [ { "type": "text", "content": "" } ], "timestamp": "2026-08-07T07:00:00.000Z",
              "metadata": { "runState": { "sessionState": { "mainAgentState": { "agentType": "base2-free" } } } } }
        ]"#,
        "deepseek/deepseek-v4-flash",
    );

    // The assistant message carries no output text, so nothing is estimated
    // even though the chat is marked as Freebuff.
    assert!(parse_freebuff_file(&path).is_empty());
}

#[test]
fn test_codebuff_chat_without_usage_is_not_attributed_to_freebuff() {
    // A Codebuff chat carries the paid root agent id in the persisted run
    // state. Plenty of real Codebuff turns record no usage at all (interrupted
    // runs, errors, turns whose credits never landed), so "has no usage" is a
    // property of the transcript, not of the product. Attributing those to
    // Freebuff puts a fabricated Freebuff row in a Codebuff-only user's
    // breakdown — and that client id feeds the submitted payload.
    let dir = TempDir::new().unwrap();
    let base = dir.path().join("manicode");
    let path = write_chat(
        &base,
        "proj",
        "2026-08-07T09-00-00.000Z",
        r#"[
            { "variant": "user", "content": "refactor this", "timestamp": "2026-08-07T09:00:00.000Z" },
            { "variant": "ai", "content": "", "blocks": [ { "type": "text", "content": "Done." } ],
              "timestamp": "2026-08-07T09:00:10.000Z",
              "metadata": { "runState": { "sessionState": { "mainAgentState": {
                  "agentType": "base2-lite" } } } } }
        ]"#,
        "deepseek/deepseek-v4-flash",
    );

    assert!(
        parse_freebuff_file(&path).is_empty(),
        "a chat whose root agent is a paid Codebuff agent must never be estimated as Freebuff"
    );
}

#[test]
fn test_freebuff_chat_is_identified_by_free_root_agent_id() {
    // Freebuff always runs a `base2-free*` root agent (its model picker maps
    // every free model onto one). That id is persisted per assistant message
    // under metadata.runState, so it is a positive per-chat marker.
    let dir = TempDir::new().unwrap();
    let base = dir.path().join("manicode");
    let path = write_chat(
        &base,
        "proj",
        "2026-08-07T10-00-00.000Z",
        r#"[
            { "variant": "user", "content": "hello world", "timestamp": "2026-08-07T10:00:00.000Z" },
            { "variant": "ai", "content": "", "blocks": [ { "type": "text", "content": "Hello!" } ],
              "timestamp": "2026-08-07T10:00:05.000Z",
              "metadata": { "runState": { "sessionState": { "mainAgentState": {
                  "agentType": "base2-free-deepseek-flash" } } } } }
        ]"#,
        "deepseek/deepseek-v4-flash",
    );

    let msgs = parse_freebuff_file(&path);
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].client, "freebuff");
}

#[test]
fn test_unmarked_chat_is_not_claimed_by_freebuff() {
    // No root agent id anywhere (a chat that never completed a turn, or one
    // written by a CLI old enough not to persist run state). Absence of a
    // Freebuff marker is not evidence of Freebuff, so claim nothing.
    let dir = TempDir::new().unwrap();
    let base = dir.path().join("manicode");
    let path = write_chat(
        &base,
        "proj",
        "2026-08-07T11-00-00.000Z",
        r#"[
            { "variant": "user", "content": "hello world", "timestamp": "2026-08-07T11:00:00.000Z" },
            { "variant": "ai", "content": "", "blocks": [ { "type": "text", "content": "Hello!" } ],
              "timestamp": "2026-08-07T11:00:05.000Z" }
        ]"#,
        "deepseek/deepseek-v4-flash",
    );

    assert!(parse_freebuff_file(&path).is_empty());
}
