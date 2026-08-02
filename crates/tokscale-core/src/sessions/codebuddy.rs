//! CodeBuddy session parser.
//!
//! CodeBuddy persists CLI/WebUI usage as JSONL transcripts under
//! `~/.codebuddy/projects/<project-key>/*.jsonl`, and the IDE / VS Code
//! extension writes final agent usage into extension logs.

use super::UnifiedMessage;
use std::path::Path;

const DEFAULT_MODEL: &str = "codebuddy";

pub fn parse_codebuddy_file(path: &Path) -> Vec<UnifiedMessage> {
    if super::tencent_buddy::is_extension_log_source(path) {
        return super::tencent_buddy::parse_extension_log_file("codebuddy", DEFAULT_MODEL, path);
    }

    super::tencent_buddy::parse_jsonl_file("codebuddy", DEFAULT_MODEL, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_codebuddy_file_reads_message_usage() {
        let dir = tempfile::tempdir().unwrap();
        let project_dir = dir.path().join("projects").join("c-Users-alice-repo");
        std::fs::create_dir_all(&project_dir).unwrap();
        let path = project_dir.join("session-1.jsonl");
        std::fs::write(
            &path,
            r#"{"id":"assistant-1","timestamp":1780000000100,"type":"message","role":"assistant","status":"completed","sessionId":"session-1","cwd":"/Users/alice/repo","providerData":{"model":"glm-5.2","messageId":"msg-1"},"message":{"usage":{"input_tokens":24486,"output_tokens":3,"cache_read_input_tokens":14720}}}"#,
        )
        .unwrap();

        let messages = parse_codebuddy_file(&path);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].client, "codebuddy");
        assert_eq!(messages[0].model_id, "glm-5.2");
        assert_eq!(messages[0].tokens.input, 24486);
        assert_eq!(messages[0].tokens.output, 3);
        assert_eq!(messages[0].tokens.cache_read, 14720);
        assert_eq!(messages[0].workspace_label.as_deref(), Some("repo"));
    }

    #[test]
    fn parse_codebuddy_file_reads_function_call_usage() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session-2.jsonl");
        std::fs::write(
            &path,
            r#"{"id":"call-1","timestamp":1780000000100,"type":"function_call","sessionId":"session-2","providerData":{"requestModelId":"minimax-m3-pay","messageId":"msg-2","rawUsage":{"prompt_tokens":10,"completion_tokens":2,"prompt_cache_hit_tokens":3}}}"#,
        )
        .unwrap();

        let messages = parse_codebuddy_file(&path);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "minimax-m3-pay");
        assert_eq!(messages[0].provider_id, "minimax");
        assert_eq!(messages[0].tokens.total(), 15);
    }

    #[test]
    fn parse_codebuddy_file_reads_extension_log_usage() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ide-extension.log");
        std::fs::write(
            &path,
            r#"[2026/7/1 16:56:01.100] [Info] [CraftInvokableAgent] [agent-1]  Model prepared: Kimi-K2.7-Code (kimi-k2.7)
[2026/7/1 16:56:02.200] [Info] [AgentReporter] [agent-1]  Agent execution successful with usage: {"inputTokens":140732,"outputTokens":635,"totalTokens":141367,"cacheTokens":76032,"cachedWriteTokens":0,"cachedMissTokens":64700,"lastTokens":71051,"credit":10.38}"#,
        )
        .unwrap();

        let messages = parse_codebuddy_file(&path);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "kimi-k2.7");
        assert_eq!(messages[0].tokens.total(), 141367);
        assert_eq!(
            messages[0].workspace_label.as_deref(),
            Some("ide-extension")
        );
    }

    #[test]
    fn parse_codebuddy_file_reads_vscode_extension_host_log_usage() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vscode-extension.log");
        std::fs::write(
            &path,
            r#"2026-07-01 17:00:31.780 [info] [CraftInvokableAgent] [agent-2] Model prepared: GLM-5v-Turbo (glm-5v-turbo)
2026-07-01 17:00:59.790 [info] [AgentReporter] [agent-2] Agent execution successful with usage: {"inputTokens":32604,"outputTokens":557,"totalTokens":33161,"cacheTokens":20841,"cachedWriteTokens":0,"cachedMissTokens":11763,"lastTokens":18141,"credit":2.6}"#,
        )
        .unwrap();

        let messages = parse_codebuddy_file(&path);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "glm-5v-turbo");
        assert_eq!(messages[0].tokens.input, 11763);
        assert_eq!(messages[0].tokens.output, 557);
        assert_eq!(messages[0].tokens.cache_read, 20841);
        assert_eq!(messages[0].tokens.total(), 33161);
    }
}
