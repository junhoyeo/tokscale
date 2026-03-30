//! KiloCode task parser
//!
//! Shares the same task-log format as Roo Code and reuses the same parser helper.

use super::roocode::parse_roo_kilo_file;
use super::UnifiedMessage;
use std::path::Path;

pub fn parse_kilocode_file(path: &Path) -> Vec<UnifiedMessage> {
    parse_roo_kilo_file(path, "kilo")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_parse_kilocode_valid_api_req_started() {
        let dir = TempDir::new().unwrap();
        let task_dir = dir.path().join("tasks").join("kilo-task-1");
        fs::create_dir_all(&task_dir).unwrap();
        fs::write(
            task_dir.join("ui_messages.json"),
            r#"[
  {
    "type": "say",
    "say": "api_req_started",
    "ts": "2026-02-18T12:00:00Z",
    "text": "{\"cost\":0.05,\"tokensIn\":40,\"tokensOut\":15,\"cacheReads\":7,\"cacheWrites\":3,\"apiProtocol\":\"openai\"}"
  }
]"#,
        )
        .unwrap();
        fs::write(
            task_dir.join("api_conversation_history.json"),
            r#"
<environment_details>
<model>gpt-5</model>
<name>KiloAgent</name>
</environment_details>
"#,
        )
        .unwrap();

        let messages = parse_kilocode_file(&task_dir.join("ui_messages.json"));
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].client, "kilo");
        assert_eq!(messages[0].provider_id, "openai");
        assert_eq!(messages[0].model_id, "gpt-5");
        assert_eq!(messages[0].session_id, "kilo-task-1");
        assert_eq!(messages[0].agent.as_deref(), Some("KiloAgent"));
        assert_eq!(messages[0].tokens.input, 40);
        assert_eq!(messages[0].tokens.output, 15);
        assert_eq!(messages[0].tokens.cache_read, 7);
        assert_eq!(messages[0].tokens.cache_write, 3);
        assert_eq!(messages[0].cost, 0.05);
    }

    #[test]
    fn test_parse_kilocode_ignores_non_api_req_started_events() {
        let dir = TempDir::new().unwrap();
        let task_dir = dir.path().join("tasks").join("kilo-task-2");
        fs::create_dir_all(&task_dir).unwrap();
        fs::write(
            task_dir.join("ui_messages.json"),
            r#"[
  {
    "type": "say",
    "say": "assistant_message",
    "ts": "2026-02-18T12:00:00Z",
    "text": "{\"cost\":0.2,\"tokensIn\":10,\"tokensOut\":1,\"cacheReads\":0,\"cacheWrites\":0,\"apiProtocol\":\"openai\"}"
  }
]"#,
        )
        .unwrap();

        let messages = parse_kilocode_file(&task_dir.join("ui_messages.json"));
        assert!(messages.is_empty());
    }

    #[test]
    fn test_parse_kilocode_preserves_nested_reseller_api_protocol() {
        let dir = TempDir::new().unwrap();
        let task_dir = dir.path().join("tasks").join("kilo-task-3");
        fs::create_dir_all(&task_dir).unwrap();
        fs::write(
            task_dir.join("ui_messages.json"),
            r#"[
  {
    "type": "say",
    "say": "api_req_started",
    "ts": "2026-02-18T12:00:00Z",
    "text": "{\"cost\":0.05,\"tokensIn\":40,\"tokensOut\":15,\"cacheReads\":7,\"cacheWrites\":3,\"apiProtocol\":\"azure/openai\"}"
  }
]"#,
        )
        .unwrap();
        fs::write(
            task_dir.join("api_conversation_history.json"),
            r#"
<environment_details>
<model>gpt-5</model>
</environment_details>
"#,
        )
        .unwrap();

        let messages = parse_kilocode_file(&task_dir.join("ui_messages.json"));
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].provider_id, "azure/openai");
    }
}
