//! Cline task parser
//!
//! Cline is the upstream project that Roo Code and Kilo forked from, so it
//! shares the same VS Code globalStorage task-log format and reuses the same
//! parser helper.

use super::roocode::parse_roo_kilo_file;
use super::UnifiedMessage;
use std::path::Path;

pub fn parse_cline_file(path: &Path) -> Vec<UnifiedMessage> {
    parse_roo_kilo_file(path, "cline")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_parse_cline_valid_api_req_started() {
        let dir = TempDir::new().unwrap();
        let task_dir = dir.path().join("tasks").join("cline-task-1");
        fs::create_dir_all(&task_dir).unwrap();
        fs::write(
            task_dir.join("ui_messages.json"),
            r#"[
  {
    "type": "say",
    "say": "api_req_started",
    "ts": "2026-02-18T12:00:00Z",
    "text": "{\"cost\":0.05,\"tokensIn\":40,\"tokensOut\":15,\"cacheReads\":7,\"cacheWrites\":3,\"apiProtocol\":\"anthropic\"}"
  }
]"#,
        )
        .unwrap();
        fs::write(
            task_dir.join("api_conversation_history.json"),
            r#"
<environment_details>
<model>claude-sonnet-4</model>
<name>ClineAgent</name>
</environment_details>
"#,
        )
        .unwrap();

        let messages = parse_cline_file(&task_dir.join("ui_messages.json"));
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].client, "cline");
        assert_eq!(messages[0].provider_id, "anthropic");
        assert_eq!(messages[0].model_id, "claude-sonnet-4");
        assert_eq!(messages[0].session_id, "cline-task-1");
        assert_eq!(messages[0].agent.as_deref(), Some("ClineAgent"));
        assert_eq!(messages[0].tokens.input, 40);
        assert_eq!(messages[0].tokens.output, 15);
        assert_eq!(messages[0].tokens.cache_read, 7);
        assert_eq!(messages[0].tokens.cache_write, 3);
        assert_eq!(messages[0].cost, 0.05);
    }

    #[test]
    fn test_parse_cline_ignores_non_api_req_started_events() {
        let dir = TempDir::new().unwrap();
        let task_dir = dir.path().join("tasks").join("cline-task-2");
        fs::create_dir_all(&task_dir).unwrap();
        fs::write(
            task_dir.join("ui_messages.json"),
            r#"[
  {
    "type": "say",
    "say": "assistant_message",
    "ts": "2026-02-18T12:00:00Z",
    "text": "{\"cost\":0.2,\"tokensIn\":10,\"tokensOut\":1,\"cacheReads\":0,\"cacheWrites\":0,\"apiProtocol\":\"anthropic\"}"
  }
]"#,
        )
        .unwrap();

        let messages = parse_cline_file(&task_dir.join("ui_messages.json"));
        assert!(messages.is_empty());
    }
}
