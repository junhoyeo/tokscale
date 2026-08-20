//! Oh My Pi (omp) session parser
//!
//! Oh My Pi is a pi-mono descendant and writes the same session JSONL record
//! format, so parsing delegates to [`super::pi::parse_pi_format_file`]; only the
//! scan root and client id differ.
//!
//! Layout is one level deeper than Pi's. A top-level session is
//! `~/.omp/agent/sessions/<encoded-cwd>/<timestamp>_<uuid>.jsonl`, and when that
//! session spawns subagents they are written as sibling files inside a directory
//! of the same name: `<timestamp>_<uuid>/<AgentName>.jsonl`, plus advisor runs
//! under `<timestamp>_<uuid>/__advisor.<name>.jsonl`. Every one of those is a
//! complete session file with its own header, so the recursive `*.jsonl` scan
//! picks them up and each is parsed independently — the same way Pi's own
//! subagent files are handled.
//!
//! The scan root is deliberately a plain `~/.omp/agent/sessions` rather than an
//! env-var root. Oh My Pi reads `PI_CODING_AGENT_DIR` for its session directory,
//! but that is the *same* variable Pi itself uses, so honoring it here would make
//! a single override point both clients at one tree and double-count it. Pi is
//! registered with a fixed home-relative root for the same reason.

use super::pi::parse_pi_format_file;
use super::UnifiedMessage;
use std::path::Path;

/// Parse an Oh My Pi JSONL session file.
pub fn parse_omp_file(path: &Path) -> Vec<UnifiedMessage> {
    parse_pi_format_file(path, "omp", "omp")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn create_test_file(content: &str) -> NamedTempFile {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(content.as_bytes()).unwrap();
        file.flush().unwrap();
        file
    }

    #[test]
    fn test_parse_omp_session_with_full_usage_record() {
        // given: the record layout omp writes today — a versioned session header
        // followed by an assistant message carrying the nested usage object.
        let content = r#"{"type":"session","version":3,"id":"01a01fc8-e982-7000-b3c1-4cca8048e34a","timestamp":"2026-08-20T15:27:35.811Z","cwd":"/tmp/workspace"}
{"type":"message","id":"b2","parentId":null,"timestamp":"2026-08-20T15:27:40.000Z","message":{"role":"assistant","model":"z-ai/glm-5.2:free","provider":"openrouter-glm-free-zdr","usage":{"input":17,"output":10,"cacheRead":0,"cacheWrite":0,"totalTokens":27}}}"#;
        let file = create_test_file(content);

        // when
        let messages = parse_omp_file(file.path());

        // then
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].client, "omp");
        assert_eq!(
            messages[0].session_id,
            "01a01fc8-e982-7000-b3c1-4cca8048e34a"
        );
        assert_eq!(messages[0].model_id, "z-ai/glm-5.2:free");
        assert_eq!(messages[0].tokens.input, 17);
        assert_eq!(messages[0].tokens.output, 10);
        assert_eq!(
            messages[0].workspace_key,
            Some("/tmp/workspace".to_string())
        );
        assert_eq!(messages[0].workspace_label, Some("workspace".to_string()));
    }

    #[test]
    fn test_parse_omp_subagent_file_is_a_standalone_session() {
        // given: a subagent transcript from `<session>/<AgentName>.jsonl`. It
        // carries its own session header, so it must parse on its own rather
        // than depending on the parent file being read first.
        let content = r#"{"type":"session","version":3,"id":"01a00742-c3eb-7000-92ab-e3294db5e528-TelegramBrokerMap","timestamp":"2026-08-15T21:10:11.179Z","cwd":"/tmp/proj"}
{"type":"message","id":"s1","parentId":null,"timestamp":"2026-08-15T21:10:20.000Z","message":{"role":"assistant","model":"claude-opus-5","provider":"anthropic","usage":{"input":12,"output":34,"cacheRead":0,"cacheWrite":0,"totalTokens":46}}}"#;
        let file = create_test_file(content);

        // when
        let messages = parse_omp_file(file.path());

        // then
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].client, "omp");
        assert_eq!(messages[0].tokens.output, 34);
    }

    #[test]
    fn test_parse_omp_falls_back_to_omp_when_provider_unrecoverable() {
        // given: an unrecognizable model with no provider falls back to the omp
        // client id rather than pi's, and the message is still counted.
        let content = r#"{"type":"session","version":3,"id":"omp_ses_fallback","timestamp":"2026-08-20T15:27:35.811Z","cwd":"/tmp"}
{"type":"message","id":"b2","parentId":null,"timestamp":"2026-08-20T15:27:40.000Z","message":{"role":"assistant","model":"totally-unrecognized-model-xyz","usage":{"input":100,"output":50,"cacheRead":0,"cacheWrite":0,"totalTokens":150}}}"#;
        let file = create_test_file(content);

        // when
        let messages = parse_omp_file(file.path());

        // then
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].provider_id, "omp");
    }

    #[test]
    fn test_parse_omp_infers_provider_from_model_before_client_fallback() {
        // given: a provider-less message whose model id names a recognizable
        // family. Inference must win over the `omp` fallback, because pricing
        // keys off the vendor rather than the harness that recorded the turn.
        let content = r#"{"type":"session","version":3,"id":"omp_ses_precedence","timestamp":"2026-08-20T15:27:35.811Z","cwd":"/tmp"}
{"type":"message","id":"b1","parentId":null,"timestamp":"2026-08-20T15:27:40.000Z","message":{"role":"assistant","model":"gpt-5","usage":{"input":1,"output":1,"cacheRead":0,"cacheWrite":0,"totalTokens":2}}}"#;
        let file = create_test_file(content);

        // when
        let messages = parse_omp_file(file.path());

        // then
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].provider_id, "openai");
    }

    #[test]
    fn test_parse_omp_keeps_complete_records_before_truncated_trailing_line() {
        // given: omp appends while a turn is streaming, so the last line of a
        // live session file can be a half-written JSON object. Usage already
        // complete above it must still be counted.
        let content = concat!(
            r#"{"type":"session","version":3,"id":"omp_ses_live","timestamp":"2026-08-20T15:27:35.811Z","cwd":"/tmp"}"#,
            "\n",
            r#"{"type":"message","id":"b2","parentId":null,"timestamp":"2026-08-20T15:27:40.000Z","message":{"role":"assistant","model":"claude-opus-5","provider":"anthropic","usage":{"input":2,"output":3,"cacheRead":0,"cacheWrite":0,"totalTokens":5}}}"#,
            "\n",
            r#"{"type":"message","id":"b3","message":{"role":"assistant""#,
        );
        let file = create_test_file(content);

        // when
        let messages = parse_omp_file(file.path());

        // then
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.input, 2);
        assert_eq!(messages[0].tokens.output, 3);
    }

    #[test]
    fn test_parse_omp_ignores_non_assistant_and_control_entries() {
        // given: user turns carry no usage, and omp interleaves control records
        // between assistant turns. Neither may produce a message.
        let content = r#"{"type":"session","version":3,"id":"omp_ses_control","timestamp":"2026-08-20T15:27:35.811Z","cwd":"/tmp"}
{"type":"message","id":"c2","parentId":null,"timestamp":"2026-08-20T15:27:40.000Z","message":{"role":"user","content":"hi"}}
{"type":"thinking_level_change","id":"c3","parentId":"c2","timestamp":"2026-08-20T15:27:41.000Z","thinkingLevel":"high"}"#;
        let file = create_test_file(content);

        // when
        let messages = parse_omp_file(file.path());

        // then
        assert!(messages.is_empty());
    }

    #[test]
    fn test_parse_omp_empty_and_headerless_files_are_empty() {
        // given
        let empty = create_test_file("");
        let headerless = create_test_file(
            r#"{"type":"message","id":"b2","parentId":null,"timestamp":"2026-08-20T15:27:40.000Z","message":{"role":"assistant","model":"claude-opus-5","provider":"anthropic","usage":{"input":1,"output":1,"cacheRead":0,"cacheWrite":0,"totalTokens":2}}}"#,
        );

        // when / then
        assert!(parse_omp_file(empty.path()).is_empty());
        assert!(parse_omp_file(headerless.path()).is_empty());
    }
}
