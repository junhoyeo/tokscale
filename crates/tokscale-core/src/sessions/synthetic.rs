//! Synthetic.new detection and Octofriend session parser
//!
//! Detects synthetic.new API usage across existing agent sessions by model/provider patterns,
//! and parses Octofriend's SQLite database when token data is available.

use super::utils::open_readonly_sqlite;
use super::UnifiedMessage;
use crate::TokenBreakdown;
use std::path::Path;

// =============================================================================
// Detection Functions (Strategy 1: detect synthetic.new usage in other sources)
// =============================================================================

/// Check if a model ID indicates synthetic.new API usage.
///
/// synthetic.new uses `hf:` prefixed model IDs in requests and may return
/// provider-prefixed model IDs like `accounts/fireworks/models/...` in responses.
pub fn is_synthetic_model(model_id: &str) -> bool {
    let lower = model_id.to_lowercase();

    // HuggingFace-style model IDs used by synthetic.new
    if lower.starts_with("hf:") {
        return true;
    }

    // Fireworks provider-prefixed responses
    if lower.starts_with("accounts/fireworks/") {
        return true;
    }

    // Together AI provider-prefixed responses
    if lower.starts_with("accounts/together/") {
        return true;
    }

    false
}

/// Check if a provider ID indicates synthetic.new API usage.
pub fn is_synthetic_provider(provider_id: &str) -> bool {
    let lower = provider_id.to_lowercase();
    matches!(
        lower.as_str(),
        "synthetic" | "glhf" | "synthetic.new" | "octofriend"
    )
}

/// Check if a message appears to have been routed through Synthetic's gateway.
pub fn is_synthetic_gateway(model_id: &str, provider_id: &str) -> bool {
    is_synthetic_model(model_id) || is_synthetic_provider(provider_id)
}

// =============================================================================
// Model name normalization (strip synthetic.new prefixes for pricing lookup)
// =============================================================================

/// Normalize a synthetic.new model ID to a standard form for pricing lookup.
/// e.g. "hf:deepseek-ai/DeepSeek-V3-0324" -> "deepseek-v3-0324"
/// e.g. "accounts/fireworks/models/deepseek-v3-0324" -> "deepseek-v3-0324"
pub fn normalize_synthetic_model(model_id: &str) -> String {
    let lower = model_id.to_lowercase();

    // Strip "hf:" prefix and org name
    if let Some(rest) = lower.strip_prefix("hf:") {
        // "hf:deepseek-ai/DeepSeek-V3-0324" -> "deepseek-v3-0324"
        if let Some((_org, model)) = rest.split_once('/') {
            return model.to_string();
        }
        return rest.to_string();
    }

    // Strip "accounts/<provider>/models/" prefix
    if let Some(rest) = lower.strip_prefix("accounts/") {
        // "accounts/fireworks/models/deepseek-v3-0324" -> "deepseek-v3-0324"
        if let Some(models_rest) = rest.split_once("/models/") {
            return models_rest.1.to_string();
        }
    }

    lower
}

/// Normalize synthetic gateway fields without changing the originating client.
pub fn normalize_synthetic_gateway_fields(model_id: &mut String, provider_id: &mut String) -> bool {
    if !is_synthetic_gateway(model_id, provider_id) {
        return false;
    }

    *model_id = normalize_synthetic_model(model_id);
    if provider_id.is_empty() || provider_id.eq_ignore_ascii_case("unknown") {
        *provider_id = "synthetic".to_string();
    }

    true
}

/// Compatibility matcher for `--synthetic` / synthetic source selection.
pub fn matches_synthetic_filter(client: &str, model_id: &str, provider_id: &str) -> bool {
    client.eq_ignore_ascii_case("synthetic") || is_synthetic_gateway(model_id, provider_id)
}

// =============================================================================
// Octofriend SQLite Parser (Strategy 2: parse Octofriend sessions)
// =============================================================================

/// Parse Octofriend's SQLite database for token usage data.
///
/// Currently Octofriend only stores input_history (no token data).
/// This function checks for token-related tables and parses them when available,
/// making it future-proof for when Octofriend adds token persistence.
pub fn parse_octofriend_sqlite(db_path: &Path) -> Vec<UnifiedMessage> {
    let Some(conn) = open_readonly_sqlite(db_path) else {
        return Vec::new();
    };

    // Check if a token-tracking table exists (future-proofing)
    // Octofriend may add tables like 'messages', 'sessions', or 'token_usage'
    let has_messages_table: bool = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name IN ('messages', 'sessions', 'token_usage')",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map(|count| count > 0)
        .unwrap_or(false);

    if !has_messages_table {
        return Vec::new();
    }

    // If messages table exists, attempt to parse it
    // This follows the OpenCode SQLite pattern:
    // SELECT id, session_id, data FROM messages WHERE role = 'assistant' AND tokens IS NOT NULL
    let mut messages = Vec::new();

    // Try 'messages' table first (most likely schema)
    if let Ok(mut stmt) = conn.prepare(
        "SELECT id, model, input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, reasoning_tokens, cost, timestamp, session_id, provider FROM messages WHERE input_tokens IS NOT NULL OR output_tokens IS NOT NULL",
    ) {
        if let Ok(rows) = stmt.query_map([], |row| {
            let id: String = row.get(0)?;
            let model_id: String = row.get::<_, String>(1).unwrap_or_default();
            let input: i64 = row.get::<_, i64>(2).unwrap_or(0);
            let output: i64 = row.get::<_, i64>(3).unwrap_or(0);
            let cache_read: i64 = row.get::<_, i64>(4).unwrap_or(0);
            let cache_write: i64 = row.get::<_, i64>(5).unwrap_or(0);
            let reasoning: i64 = row.get::<_, i64>(6).unwrap_or(0);
            let cost: f64 = row.get::<_, f64>(7).unwrap_or(0.0);
            let timestamp: f64 = row.get::<_, f64>(8).unwrap_or(0.0);
            let session_id: String = row.get::<_, String>(9).unwrap_or_else(|_| "unknown".to_string());
            let provider: String = row.get::<_, String>(10).unwrap_or_else(|_| "synthetic".to_string());

            Ok((id, model_id, input, output, cache_read, cache_write, reasoning, cost, timestamp, session_id, provider))
        }) {
            for row_result in rows.flatten() {
                let (id, model_id, input, output, cache_read, cache_write, reasoning, cost, timestamp, session_id, provider) = row_result;

                let total = input + output + cache_read + cache_write + reasoning;
                if total == 0 {
                    continue;
                }

                let ts_ms = if timestamp > 1e12 {
                    timestamp as i64
                } else {
                    (timestamp * 1000.0) as i64
                };

                let mut msg = UnifiedMessage::new(
                    "synthetic",
                    normalize_synthetic_model(&model_id),
                    provider,
                    session_id,
                    ts_ms,
                    TokenBreakdown {
                        input: input.max(0),
                        output: output.max(0),
                        cache_read: cache_read.max(0),
                        cache_write: cache_write.max(0),
                        reasoning: reasoning.max(0),
                    },
                    cost.max(0.0),
                );
                msg.dedup_key = Some(id);
                messages.push(msg);
            }
        }
    }

    // Try 'token_usage' table as alternative schema
    if messages.is_empty() {
        if let Ok(mut stmt) = conn.prepare(
            "SELECT id, model, input_tokens, output_tokens, timestamp, session_id FROM token_usage WHERE input_tokens > 0 OR output_tokens > 0",
        ) {
            if let Ok(rows) = stmt.query_map([], |row| {
                let id: String = row.get(0)?;
                let model_id: String = row.get::<_, String>(1).unwrap_or_default();
                let input: i64 = row.get::<_, i64>(2).unwrap_or(0);
                let output: i64 = row.get::<_, i64>(3).unwrap_or(0);
                let timestamp: f64 = row.get::<_, f64>(4).unwrap_or(0.0);
                let session_id: String = row.get::<_, String>(5).unwrap_or_else(|_| "unknown".to_string());

                Ok((id, model_id, input, output, timestamp, session_id))
            }) {
                for row_result in rows.flatten() {
                    let (id, model_id, input, output, timestamp, session_id) = row_result;

                    let ts_ms = if timestamp > 1e12 {
                        timestamp as i64
                    } else {
                        (timestamp * 1000.0) as i64
                    };

                    let mut msg = UnifiedMessage::new(
                        "synthetic",
                        normalize_synthetic_model(&model_id),
                        "synthetic",
                        session_id,
                        ts_ms,
                        TokenBreakdown {
                            input: input.max(0),
                            output: output.max(0),
                            cache_read: 0,
                            cache_write: 0,
                            reasoning: 0,
                        },
                        0.0,
                    );
                    msg.dedup_key = Some(id);
                    messages.push(msg);
                }
            }
        }
    }

    messages
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_synthetic_model_hf_prefix() {
        assert!(is_synthetic_model("hf:deepseek-ai/DeepSeek-V3-0324"));
        assert!(is_synthetic_model("hf:zai-org/GLM-4.7"));
        assert!(is_synthetic_model("hf:moonshotai/Kimi-K2.5"));
        assert!(is_synthetic_model("hf:MiniMaxAI/MiniMax-M2.1"));
    }

    #[test]
    fn test_is_synthetic_model_fireworks_prefix() {
        assert!(is_synthetic_model(
            "accounts/fireworks/models/deepseek-v3-0324"
        ));
        assert!(is_synthetic_model("accounts/fireworks/models/glm-4.7"));
    }

    #[test]
    fn test_is_synthetic_model_together_prefix() {
        assert!(is_synthetic_model("accounts/together/models/qwen3-235b"));
    }

    #[test]
    fn test_is_synthetic_model_negative() {
        assert!(!is_synthetic_model("claude-sonnet-4-5"));
        assert!(!is_synthetic_model("gpt-5.2-codex"));
        assert!(!is_synthetic_model("deepseek-v3"));
        assert!(!is_synthetic_model("gemini-2.5-pro"));
    }

    #[test]
    fn test_is_synthetic_provider() {
        assert!(is_synthetic_provider("synthetic"));
        assert!(is_synthetic_provider("glhf"));
        assert!(is_synthetic_provider("Synthetic"));
        assert!(is_synthetic_provider("GLHF"));
        assert!(is_synthetic_provider("synthetic.new"));
        assert!(is_synthetic_provider("octofriend"));
    }

    #[test]
    fn test_is_synthetic_provider_negative() {
        assert!(!is_synthetic_provider("anthropic"));
        assert!(!is_synthetic_provider("openai"));
        assert!(!is_synthetic_provider("moonshot"));
        assert!(!is_synthetic_provider("fireworks"));
    }

    #[test]
    fn test_normalize_synthetic_model_hf() {
        assert_eq!(
            normalize_synthetic_model("hf:deepseek-ai/DeepSeek-V3-0324"),
            "deepseek-v3-0324"
        );
        assert_eq!(normalize_synthetic_model("hf:zai-org/GLM-4.7"), "glm-4.7");
        assert_eq!(
            normalize_synthetic_model("hf:moonshotai/Kimi-K2.5"),
            "kimi-k2.5"
        );
    }

    #[test]
    fn test_normalize_synthetic_model_fireworks() {
        assert_eq!(
            normalize_synthetic_model("accounts/fireworks/models/deepseek-v3-0324"),
            "deepseek-v3-0324"
        );
    }

    #[test]
    fn test_normalize_synthetic_model_passthrough() {
        assert_eq!(
            normalize_synthetic_model("claude-sonnet-4-5"),
            "claude-sonnet-4-5"
        );
        assert_eq!(normalize_synthetic_model("gpt-4o"), "gpt-4o");
    }

    #[test]
    fn test_normalize_synthetic_gateway_fields_sets_provider_when_unknown() {
        let mut model_id = "hf:deepseek-ai/DeepSeek-V3-0324".to_string();
        let mut provider_id = "unknown".to_string();

        let matched = normalize_synthetic_gateway_fields(&mut model_id, &mut provider_id);

        assert!(matched);
        assert_eq!(model_id, "deepseek-v3-0324");
        assert_eq!(provider_id, "synthetic");
    }

    #[test]
    fn test_normalize_synthetic_gateway_fields_preserves_existing_provider() {
        let mut model_id = "accounts/fireworks/models/deepseek-v3-0324".to_string();
        let mut provider_id = "fireworks".to_string();

        let matched = normalize_synthetic_gateway_fields(&mut model_id, &mut provider_id);

        assert!(matched);
        assert_eq!(model_id, "deepseek-v3-0324");
        assert_eq!(provider_id, "fireworks");
    }

    #[test]
    fn test_matches_synthetic_filter_accepts_gateway_traffic_without_rewriting_client() {
        assert!(matches_synthetic_filter(
            "opencode",
            "hf:deepseek-ai/DeepSeek-V3-0324",
            "unknown"
        ));
        assert!(matches_synthetic_filter(
            "claude",
            "claude-sonnet-4-5",
            "glhf"
        ));
        assert!(!matches_synthetic_filter("opencode", "gpt-4o", "anthropic"));
    }

    #[test]
    fn test_parse_octofriend_sqlite_nonexistent() {
        let result = parse_octofriend_sqlite(Path::new("/nonexistent/path/sqlite.db"));
        assert!(result.is_empty());
    }
}
