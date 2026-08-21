//! Kimchi Coding session parser.
//!
//! Kimchi stores sessions in the Pi-compatible JSONL format under its own
//! agent directory. Reuse the shared Pi parser while stamping messages with
//! the distinct `kimchi` client id.

use super::pi::parse_pi_format_file_with_dedup;
use super::UnifiedMessage;
use std::path::Path;

pub fn parse_kimchi_file(path: &Path) -> Vec<UnifiedMessage> {
    parse_pi_format_file_with_dedup(path, "kimchi", "kimchi")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_parse_kimchi_pi_format_session() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(
            file,
            r#"{{"type":"session","id":"kimchi_ses_001","timestamp":"2026-08-01T00:00:00.000Z","cwd":"/tmp/kimchi-project"}}"#
        )
        .unwrap();
        writeln!(
            file,
            r#"{{"type":"message","id":"msg_001","timestamp":"2026-08-01T00:00:01.000Z","message":{{"role":"assistant","model":"kimi-k2.6","provider":"kimchi-dev","usage":{{"input":9441,"output":131,"cacheRead":50,"cacheWrite":0,"totalTokens":9622}}}}}}"#
        )
        .unwrap();

        let messages = parse_kimchi_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].client, "kimchi");
        assert_eq!(messages[0].session_id, "kimchi_ses_001");
        assert_eq!(messages[0].provider_id, "kimchi-dev");
        assert_eq!(messages[0].model_id, "kimi-k2.6");
        assert_eq!(messages[0].tokens.input, 9441);
        assert_eq!(messages[0].tokens.output, 131);
        assert_eq!(messages[0].tokens.cache_read, 50);
        assert_eq!(
            messages[0].dedup_key.as_deref(),
            Some("kimchi:kimchi_ses_001:msg_001")
        );
        assert_eq!(
            messages[0].workspace_label.as_deref(),
            Some("kimchi-project")
        );
    }
}
