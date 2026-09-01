//! Unsloth Studio inference usage parser.
//!
//! Unsloth Studio stores durable chat and API usage in `studio.db` beneath
//! `$UNSLOTH_STUDIO_HOME` (normally `~/.unsloth/studio`). Internal Studio
//! responses keep usage in assistant-message metadata; authenticated external
//! API requests are copied into the content-free `api_usage_events` table.
//! `responseDetails.providerType` distinguishes local zero-cost inference from
//! recognized metered providers; routes without reliable billing identity stay
//! unpriced. Neither query selects message content, prompts, or response previews.

use super::utils::{open_readonly_sqlite_opt, sqlite_for_each_row_on, timestamp_secs_to_ms};
use super::UnifiedMessage;
use crate::{provider_identity, TokenBreakdown};
use rusqlite::Connection;
use std::path::Path;

const CLIENT_ID: &str = "unsloth";
const STUDIO_AGENT: &str = "Unsloth";
const API_AGENT: &str = "Unsloth API";

const METERED_PROVIDER_TYPES: &[&str] = &[
    "anthropic",
    "deepseek",
    "gemini",
    "huggingface",
    "kimi",
    "mistral",
    "openai",
    "openrouter",
    "qwen",
];

fn non_blank(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    })
}

fn provider_for_pricing(provider_type: Option<String>) -> (String, bool) {
    let provider_type = non_blank(provider_type)
        .map(|provider| provider.to_ascii_lowercase())
        .unwrap_or_else(|| "unknown".to_string());
    if provider_type == "local" {
        return (provider_type, true);
    }
    if METERED_PROVIDER_TYPES.contains(&provider_type.as_str()) {
        // Unsloth names the Kimi route by its product brand, while TokScale's
        // metered catalog uses Moonshot AI as the canonical provider.
        let provider = if provider_type == "kimi" {
            "moonshotai".to_string()
        } else {
            provider_identity::canonical_provider(&provider_type)
                .unwrap_or_else(|| provider_type.clone())
        };
        return (provider, false);
    }

    // Custom, subscription-backed, and future routes do not carry enough
    // billing metadata to justify an upstream model-price estimate. The
    // reserved provider namespace keeps the responding model visible while
    // making the usage explicitly unpriceable unless the user supplies an
    // exact custom-pricing override for that model.
    (format!("unpriced:{provider_type}"), false)
}

fn normalized_tokens(
    prompt_tokens: i64,
    completion_tokens: i64,
    total_tokens: i64,
    cached_tokens: i64,
    cache_write_tokens: i64,
    reasoning_tokens: i64,
) -> Option<TokenBreakdown> {
    let prompt = prompt_tokens.max(0);
    let completion = completion_tokens.max(0);
    let cache_read = cached_tokens.max(0).min(prompt);
    let cache_write = cache_write_tokens
        .max(0)
        .min(prompt.saturating_sub(cache_read));
    let reasoning = reasoning_tokens.max(0).min(completion);
    let total = total_tokens.max(0).max(prompt.saturating_add(completion));
    if total == 0 {
        return None;
    }

    Some(TokenBreakdown {
        input: total
            .saturating_sub(completion)
            .saturating_sub(cache_read)
            .saturating_sub(cache_write),
        output: completion.saturating_sub(reasoning),
        cache_read,
        cache_write,
        reasoning,
    })
}

fn parse_internal_chat_usage(db_path: &Path, conn: &Connection) -> Vec<UnifiedMessage> {
    // SQLite projects only the scalar usage and routing fields used below.
    // `content_json`, attachments, private metadata, prompts, and response
    // previews never cross the database boundary.
    let query = r#"
        SELECT
            m.id,
            m.thread_id,
            m.created_at,
            CASE WHEN json_valid(m.metadata_json)
                THEN json_extract(m.metadata_json, '$.contextUsage.promptTokens') END,
            CASE WHEN json_valid(m.metadata_json)
                THEN json_extract(m.metadata_json, '$.contextUsage.completionTokens') END,
            CASE WHEN json_valid(m.metadata_json)
                THEN json_extract(m.metadata_json, '$.contextUsage.totalTokens') END,
            CASE WHEN json_valid(m.metadata_json)
                THEN json_extract(m.metadata_json, '$.contextUsage.cachedTokens') END,
            CASE WHEN json_valid(m.metadata_json)
                THEN json_extract(m.metadata_json, '$.contextUsage.cacheWriteTokens') END,
            CASE WHEN json_valid(m.metadata_json)
                THEN json_extract(m.metadata_json, '$.contextUsage.reasoningTokens') END,
            CASE WHEN json_valid(m.metadata_json)
                THEN json_extract(m.metadata_json, '$.contextUsage.modelId') END,
            CASE WHEN json_valid(m.metadata_json)
                THEN json_extract(m.metadata_json, '$.responseDetails.responseModelId') END,
            CASE WHEN json_valid(m.metadata_json)
                THEN json_extract(m.metadata_json, '$.responseDetails.providerType') END,
            t.model_id
        FROM chat_messages m
        LEFT JOIN chat_threads t ON t.id = m.thread_id
        WHERE m.role = 'assistant'
        ORDER BY m.created_at, m.id
        "#;

    let mut messages = Vec::new();
    sqlite_for_each_row_on(
        conn,
        db_path,
        query,
        Some("Unsloth Studio chat usage"),
        &mut |row| {
            let message_id: String = row.get(0)?;
            let thread_id: String = row.get(1)?;
            let created_at: i64 = row.get(2)?;
            let prompt_tokens: Option<i64> = row.get(3)?;
            let completion_tokens: Option<i64> = row.get(4)?;
            let total_tokens: Option<i64> = row.get(5)?;
            let cached_tokens: Option<i64> = row.get(6)?;
            let cache_write_tokens: Option<i64> = row.get(7)?;
            let reasoning_tokens: Option<i64> = row.get(8)?;
            let requested_model: Option<String> = row.get(9)?;
            let response_model: Option<String> = row.get(10)?;
            let provider_type: Option<String> = row.get(11)?;
            let thread_model: Option<String> = row.get(12)?;

            let Some(tokens) = normalized_tokens(
                prompt_tokens.unwrap_or_default(),
                completion_tokens.unwrap_or_default(),
                total_tokens.unwrap_or_default(),
                cached_tokens.unwrap_or_default(),
                cache_write_tokens.unwrap_or_default(),
                reasoning_tokens.unwrap_or_default(),
            ) else {
                return Ok(());
            };
            let timestamp = timestamp_secs_to_ms(created_at as f64);
            if timestamp <= 0 || message_id.trim().is_empty() {
                return Ok(());
            }

            let model = non_blank(response_model)
                .or_else(|| non_blank(requested_model))
                .or_else(|| non_blank(thread_model))
                .unwrap_or_else(|| "unknown".to_string());
            let (provider, authoritative_zero_cost) = provider_for_pricing(provider_type);
            let session_id = if thread_id.trim().is_empty() {
                format!("unsloth:chat:{message_id}")
            } else {
                thread_id
            };
            let mut message = UnifiedMessage::new_with_dedup(
                CLIENT_ID,
                model,
                provider,
                session_id,
                timestamp,
                tokens,
                0.0,
                Some(format!("unsloth:chat:{message_id}")),
            );
            message.agent = Some(STUDIO_AGENT.to_string());
            message.is_turn_start = true;
            if authoritative_zero_cost {
                message.mark_provider_reported_cost();
            }
            messages.push(message);
            Ok(())
        },
    );
    messages
}

fn parse_external_api_usage(db_path: &Path, conn: &Connection) -> Vec<UnifiedMessage> {
    // Older Studio builds do not have this table. Probe silently so their
    // internal chat usage still imports without a warning on every scan.
    let query = r#"
        SELECT
            id,
            endpoint,
            model,
            prompt_tokens,
            completion_tokens,
            total_tokens,
            created_at
        FROM api_usage_events
        ORDER BY created_at, id
        "#;

    let mut messages = Vec::new();
    sqlite_for_each_row_on(conn, db_path, query, None, &mut |row| {
        let id: String = row.get(0)?;
        let endpoint: String = row.get(1)?;
        let model: String = row.get(2)?;
        let prompt_tokens: i64 = row.get(3)?;
        let completion_tokens: i64 = row.get(4)?;
        let total_tokens: i64 = row.get(5)?;
        let created_at: i64 = row.get(6)?;

        let Some(tokens) =
            normalized_tokens(prompt_tokens, completion_tokens, total_tokens, 0, 0, 0)
        else {
            return Ok(());
        };
        let timestamp = timestamp_secs_to_ms(created_at as f64);
        if timestamp <= 0 || id.trim().is_empty() {
            return Ok(());
        }

        let model = non_blank(Some(model)).unwrap_or_else(|| "unknown".to_string());
        let mut message = UnifiedMessage::new_with_dedup(
            CLIENT_ID,
            model,
            "local",
            "unsloth:api".to_string(),
            timestamp,
            tokens,
            0.0,
            Some(format!("unsloth:api:{id}")),
        );
        message.agent = Some(API_AGENT.to_string());
        message.session_title = non_blank(Some(endpoint));
        message.is_turn_start = true;
        message.mark_provider_reported_cost();
        messages.push(message);
        Ok(())
    });
    messages
}

/// Parse durable Unsloth Studio inference usage without selecting chat content.
pub fn parse_unsloth_sqlite(db_path: &Path) -> Vec<UnifiedMessage> {
    let Some(conn) = open_readonly_sqlite_opt(db_path) else {
        return Vec::new();
    };

    let mut messages = parse_internal_chat_usage(db_path, &conn);
    messages.extend(parse_external_api_usage(db_path, &conn));
    messages
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::{params, Connection};

    fn create_database(path: &Path, include_api_usage: bool) -> Connection {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE chat_threads (
                id TEXT PRIMARY KEY,
                model_id TEXT
            );
            CREATE TABLE chat_messages (
                id TEXT PRIMARY KEY,
                thread_id TEXT NOT NULL,
                role TEXT NOT NULL,
                metadata_json TEXT,
                created_at INTEGER NOT NULL
            );
            "#,
        )
        .unwrap();
        if include_api_usage {
            conn.execute_batch(
                r#"
                CREATE TABLE api_usage_events (
                    id TEXT PRIMARY KEY,
                    subject TEXT NOT NULL,
                    endpoint TEXT NOT NULL,
                    model TEXT NOT NULL,
                    status TEXT NOT NULL,
                    prompt_tokens INTEGER NOT NULL,
                    completion_tokens INTEGER NOT NULL,
                    total_tokens INTEGER NOT NULL,
                    created_at INTEGER NOT NULL
                );
                "#,
            )
            .unwrap();
        }
        conn
    }

    #[test]
    fn returns_empty_for_missing_database() {
        let dir = tempfile::tempdir().unwrap();
        assert!(parse_unsloth_sqlite(&dir.path().join("missing.db")).is_empty());
    }

    #[test]
    fn classifies_local_metered_and_unknown_provider_routes() {
        assert_eq!(
            provider_for_pricing(Some(" local ".to_string())),
            ("local".to_string(), true)
        );
        for (provider_type, canonical_provider) in [
            ("anthropic", "anthropic"),
            ("deepseek", "deepseek"),
            ("Gemini", "google"),
            ("huggingface", "huggingface"),
            ("kimi", "moonshotai"),
            ("mistral", "mistralai"),
            ("openai", "openai"),
            ("openrouter", "openrouter"),
            ("qwen", "qwen"),
        ] {
            assert_eq!(
                provider_for_pricing(Some(provider_type.to_string())),
                (canonical_provider.to_string(), false),
                "provider type: {provider_type}"
            );
        }
        assert_eq!(
            provider_for_pricing(Some("custom".to_string())),
            ("unpriced:custom".to_string(), false)
        );
        assert_eq!(
            provider_for_pricing(None),
            ("unpriced:unknown".to_string(), false)
        );
    }

    #[test]
    fn parses_internal_chat_and_content_free_api_usage() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("studio.db");
        let conn = create_database(&db_path, true);
        conn.execute(
            "INSERT INTO chat_threads (id, model_id) VALUES (?1, ?2)",
            params!["thread-1", "thread-fallback"],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO chat_messages (id, thread_id, role, metadata_json, created_at) VALUES (?1, ?2, 'assistant', ?3, ?4)",
            params![
                "message-1",
                "thread-1",
                r#"{"privatePreview":"never select this","contextUsage":{"promptTokens":100,"completionTokens":40,"totalTokens":140,"cachedTokens":30,"cacheWriteTokens":10,"reasoningTokens":5,"modelId":"requested-model"},"responseDetails":{"responseModelId":"claude-sonnet-4-6","providerType":"anthropic"}}"#,
                1_788_000_000_123_i64,
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO api_usage_events (id, subject, endpoint, model, status, prompt_tokens, completion_tokens, total_tokens, created_at) VALUES (?1, ?2, ?3, ?4, 'completed', ?5, ?6, ?7, ?8)",
            params![
                "request-1",
                "private-user",
                "/v1/chat/completions",
                "unsloth/local-api-model",
                20_i64,
                7_i64,
                27_i64,
                1_788_000_100_i64,
            ],
        )
        .unwrap();
        drop(conn);

        let messages = parse_unsloth_sqlite(&db_path);
        assert_eq!(messages.len(), 2);

        let chat = &messages[0];
        assert_eq!(chat.model_id, "claude-sonnet-4-6");
        assert_eq!(chat.provider_id, "anthropic");
        assert_eq!(chat.session_id, "thread-1");
        assert_eq!(chat.timestamp, 1_788_000_000_123);
        assert_eq!(chat.tokens.input, 60);
        assert_eq!(chat.tokens.output, 35);
        assert_eq!(chat.tokens.cache_read, 30);
        assert_eq!(chat.tokens.cache_write, 10);
        assert_eq!(chat.tokens.reasoning, 5);
        assert_eq!(chat.tokens.total(), 140);
        assert_eq!(chat.dedup_key.as_deref(), Some("unsloth:chat:message-1"));
        assert_eq!(chat.agent.as_deref(), Some(STUDIO_AGENT));
        assert!(chat.is_turn_start);
        assert_eq!(chat.cost, 0.0);
        assert_eq!(chat.cost_source, super::super::CostSource::Unknown);

        let api = &messages[1];
        assert_eq!(api.model_id, "unsloth/local-api-model");
        assert_eq!(api.timestamp, 1_788_000_100_000);
        assert_eq!(api.tokens.input, 20);
        assert_eq!(api.tokens.output, 7);
        assert_eq!(api.tokens.total(), 27);
        assert_eq!(api.agent.as_deref(), Some(API_AGENT));
        assert_eq!(api.session_title.as_deref(), Some("/v1/chat/completions"));
        assert_eq!(api.dedup_key.as_deref(), Some("unsloth:api:request-1"));
        assert_eq!(api.session_id, "unsloth:api");
        assert_eq!(api.provider_id, "local");
        assert_eq!(api.cost_source, super::super::CostSource::ProviderReported);
    }

    #[test]
    fn older_schema_without_api_table_still_parses_chat_usage() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("studio.db");
        let conn = create_database(&db_path, false);
        conn.execute(
            "INSERT INTO chat_threads (id, model_id) VALUES ('thread-1', 'fallback-model')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO chat_messages (id, thread_id, role, metadata_json, created_at) VALUES (?1, ?2, 'assistant', ?3, ?4)",
            params![
                "message-1",
                "thread-1",
                r#"{"contextUsage":{"promptTokens":5,"completionTokens":3,"totalTokens":8}}"#,
                1_788_000_000_i64,
            ],
        )
        .unwrap();
        drop(conn);

        let messages = parse_unsloth_sqlite(&db_path);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "fallback-model");
        assert_eq!(messages[0].provider_id, "unpriced:unknown");
        assert_eq!(messages[0].tokens.total(), 8);
    }

    #[test]
    fn local_routes_are_free_and_unknown_routes_remain_unpriced() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("studio.db");
        let conn = create_database(&db_path, false);
        conn.execute(
            "INSERT INTO chat_threads (id, model_id) VALUES ('thread-1', 'fallback-model')",
            [],
        )
        .unwrap();
        for (id, metadata) in [
            (
                "local-message",
                r#"{"contextUsage":{"promptTokens":10,"completionTokens":2,"totalTokens":12},"responseDetails":{"responseModelId":"local-model","providerType":"local"}}"#,
            ),
            (
                "custom-message",
                r#"{"contextUsage":{"promptTokens":20,"completionTokens":4,"totalTokens":24},"responseDetails":{"responseModelId":"gpt-5.4","providerType":"custom"}}"#,
            ),
        ] {
            conn.execute(
                "INSERT INTO chat_messages (id, thread_id, role, metadata_json, created_at) VALUES (?1, 'thread-1', 'assistant', ?2, ?3)",
                params![id, metadata, 1_788_000_000_i64],
            )
            .unwrap();
        }
        drop(conn);

        let messages = parse_unsloth_sqlite(&db_path);
        assert_eq!(messages.len(), 2);

        let custom = messages
            .iter()
            .find(|message| message.dedup_key.as_deref() == Some("unsloth:chat:custom-message"))
            .unwrap();
        assert_eq!(custom.model_id, "gpt-5.4");
        assert_eq!(custom.provider_id, "unpriced:custom");
        assert_eq!(custom.cost_source, super::super::CostSource::Unknown);

        let local = messages
            .iter()
            .find(|message| message.dedup_key.as_deref() == Some("unsloth:chat:local-message"))
            .unwrap();
        assert_eq!(local.model_id, "local-model");
        assert_eq!(local.provider_id, "local");
        assert_eq!(
            local.cost_source,
            super::super::CostSource::ProviderReported
        );
    }

    #[test]
    fn skips_user_messages_malformed_metadata_and_zero_usage() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("studio.db");
        let conn = create_database(&db_path, false);
        for (id, role, metadata) in [
            (
                "user-1",
                "user",
                r#"{"contextUsage":{"promptTokens":10,"totalTokens":10}}"#,
            ),
            ("assistant-bad", "assistant", "not-json"),
            (
                "assistant-zero",
                "assistant",
                r#"{"contextUsage":{"promptTokens":0,"completionTokens":0,"totalTokens":0}}"#,
            ),
        ] {
            conn.execute(
                "INSERT INTO chat_messages (id, thread_id, role, metadata_json, created_at) VALUES (?1, 'thread-1', ?2, ?3, ?4)",
                params![id, role, metadata, 1_788_000_000_i64],
            )
            .unwrap();
        }
        drop(conn);

        assert!(parse_unsloth_sqlite(&db_path).is_empty());
    }
}
