//! Senpi (OmO Native) session parser
//!
//! Senpi is a pi-mono descendant using the same JSONL record format, so parsing
//! delegates to [`super::pi::parse_pi_format_file`]; only the scan root and
//! client id differ. Two divergences from Pi matter here: `usage.reasoning` is
//! parsed but never summed, because senpi documents it as a subset of `output`
//! while tokscale totals reasoning as its own additive bucket; and
//! `session_info.name` carries a human session title rather than Pi's
//! `subagent-<name>-<id>` marker.
//!
//! OmO task children are senpi sessions too. The scanner honors
//! `SENPI_CODING_AGENT_SESSION_DIR` and discovers the current project's
//! `.omo/senpi-task/children` tree so their redirected sessions are counted.

use super::pi::parse_pi_format_file;
use super::UnifiedMessage;
use std::path::Path;

/// Parse a Senpi JSONL session file.
pub fn parse_senpi_file(path: &Path) -> Vec<UnifiedMessage> {
    parse_pi_format_file(path, "senpi", "senpi")
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
    fn test_parse_senpi_v3_session_with_full_usage_record() {
        // given: the record layout senpi writes today — a versioned header, a
        // model_change entry, then an assistant message whose usage carries the
        // nested cost object and cacheWrite1h/reasoning fields.
        let content = r#"{"type":"session","version":3,"id":"019fae75-f35c-7b20-8d6f-e6dea8f7d9f5","timestamp":"2026-07-29T15:19:53.436Z","cwd":"/tmp/workspace"}
{"type":"model_change","id":"a1","parentId":null,"timestamp":"2026-07-29T15:19:53.500Z","provider":"anthropic","modelId":"claude-opus-5"}
{"type":"message","id":"b2","parentId":"a1","timestamp":"2026-07-29T15:20:01.000Z","message":{"role":"assistant","model":"claude-opus-5","provider":"anthropic","api":"anthropic-messages","responseId":"resp_1","stopReason":"stop","usage":{"input":2,"output":49,"cacheRead":40625,"cacheWrite":332,"totalTokens":41008,"cacheWrite1h":0,"reasoning":16,"cost":{"input":0.00001,"output":0.001225,"cacheRead":0.0203125,"cacheWrite":0.002075,"total":0.0236225}}}}"#;
        let file = create_test_file(content);

        // when
        let messages = parse_senpi_file(file.path());

        // then
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].client, "senpi");
        assert_eq!(
            messages[0].session_id,
            "019fae75-f35c-7b20-8d6f-e6dea8f7d9f5"
        );
        assert_eq!(messages[0].model_id, "claude-opus-5");
        assert_eq!(messages[0].provider_id, "anthropic");
        assert_eq!(messages[0].tokens.input, 2);
        assert_eq!(messages[0].tokens.output, 49);
        assert_eq!(messages[0].tokens.cache_read, 40625);
        assert_eq!(messages[0].tokens.cache_write, 332);
        assert_eq!(
            messages[0].workspace_key,
            Some("/tmp/workspace".to_string())
        );
        assert_eq!(messages[0].workspace_label, Some("workspace".to_string()));
    }

    #[test]
    fn test_parse_senpi_does_not_double_count_reasoning_into_output() {
        // given: senpi documents usage.reasoning as a subset of usage.output,
        // while tokscale totals reasoning as its own additive bucket. Mapping
        // the field through would inflate the total, so it must stay zero and
        // output must stay exactly as reported. `PiUsage` deserializes
        // `reasoning`, so mapping it is a one-line change that this test catches.
        let content = r#"{"type":"session","version":3,"id":"senpi_ses_reasoning","timestamp":"2026-07-29T15:19:53.436Z","cwd":"/tmp"}
{"type":"message","id":"b2","parentId":null,"timestamp":"2026-07-29T15:20:01.000Z","message":{"role":"assistant","model":"claude-opus-5","provider":"anthropic","usage":{"input":2,"output":43,"cacheRead":0,"cacheWrite":38241,"totalTokens":38286,"reasoning":16}}}"#;
        let file = create_test_file(content);

        // when
        let messages = parse_senpi_file(file.path());

        // then
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.output, 43);
        assert_eq!(messages[0].tokens.reasoning, 0);
        // Mirrors the session's own totalTokens: input + output + cacheRead + cacheWrite.
        assert_eq!(
            messages[0].tokens.input
                + messages[0].tokens.output
                + messages[0].tokens.cache_read
                + messages[0].tokens.cache_write
                + messages[0].tokens.reasoning,
            38286
        );
    }

    #[test]
    fn test_parse_senpi_session_title_is_not_treated_as_agent() {
        // given: senpi writes a generated human title into session_info.name,
        // unlike pi's "subagent-<name>-<id>" marker. It must not leak into
        // agent attribution.
        let content = r#"{"type":"session","version":3,"id":"senpi_ses_title","timestamp":"2026-07-29T15:19:53.436Z","cwd":"/tmp"}
{"type":"session_info","id":"c3","parentId":"b2","timestamp":"2026-07-29T15:23:22.174Z","name":"Investigate Senpi text streaming rendering"}
{"type":"message","id":"d4","parentId":"c3","timestamp":"2026-07-29T15:23:30.000Z","message":{"role":"assistant","model":"claude-opus-5","provider":"anthropic","usage":{"input":10,"output":5,"cacheRead":0,"cacheWrite":0,"totalTokens":15}}}"#;
        let file = create_test_file(content);

        // when
        let messages = parse_senpi_file(file.path());

        // then
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].agent, None);
    }

    #[test]
    fn test_parse_senpi_infers_provider_from_model_when_absent() {
        // given
        let content = r#"{"type":"session","version":3,"id":"senpi_ses_infer","timestamp":"2026-07-29T15:19:53.436Z","cwd":"/tmp"}
{"type":"message","id":"b2","parentId":null,"timestamp":"2026-07-29T15:20:01.000Z","message":{"role":"assistant","model":"gpt-5","usage":{"input":100,"output":50,"cacheRead":0,"cacheWrite":0,"totalTokens":150}}}"#;
        let file = create_test_file(content);

        // when
        let messages = parse_senpi_file(file.path());

        // then
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].provider_id, "openai");
    }

    #[test]
    fn test_parse_senpi_falls_back_to_senpi_when_provider_unrecoverable() {
        // given: an unrecognizable model with no provider falls back to the
        // senpi client id rather than pi's, and the message is still counted.
        let content = r#"{"type":"session","version":3,"id":"senpi_ses_fallback","timestamp":"2026-07-29T15:19:53.436Z","cwd":"/tmp"}
{"type":"message","id":"b2","parentId":null,"timestamp":"2026-07-29T15:20:01.000Z","message":{"role":"assistant","model":"totally-unrecognized-model-xyz","usage":{"input":100,"output":50,"cacheRead":0,"cacheWrite":0,"totalTokens":150}}}"#;
        let file = create_test_file(content);

        // when
        let messages = parse_senpi_file(file.path());

        // then
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].provider_id, "senpi");
    }

    #[test]
    fn test_parse_senpi_provider_inference_wins_over_the_client_fallback() {
        // given: a provider-less message whose model id carries a recognizable
        // family token. Inference runs before the `senpi` fallback on purpose --
        // an id that names a family is better attributed to that vendor than to
        // the harness that recorded it, because pricing keys off the vendor.
        let content = r#"{"type":"session","version":3,"id":"senpi_ses_precedence","timestamp":"2026-07-29T15:19:53.436Z","cwd":"/tmp"}
{"type":"message","id":"b1","parentId":null,"timestamp":"2026-07-29T15:20:01.000Z","message":{"role":"assistant","model":"qwen3-coder","usage":{"input":1,"output":1,"cacheRead":0,"cacheWrite":0,"totalTokens":2}}}
{"type":"message","id":"b2","parentId":"b1","timestamp":"2026-07-29T15:20:02.000Z","message":{"role":"assistant","model":"internal-house-model","usage":{"input":1,"output":1,"cacheRead":0,"cacheWrite":0,"totalTokens":2}}}"#;
        let file = create_test_file(content);

        // when
        let messages = parse_senpi_file(file.path());

        // then: only a genuinely unrecognizable id lands on the client fallback.
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].provider_id, "qwen");
        assert_eq!(messages[1].provider_id, "senpi");
    }

    #[test]
    fn test_parse_senpi_ignores_custom_and_non_assistant_entries() {
        // given: omo injects `custom` records (e.g. the ultrawork directive)
        // and user turns carry no usage. Neither may produce a message.
        let content = r#"{"type":"session","version":3,"id":"senpi_ses_custom","timestamp":"2026-07-29T15:19:53.436Z","cwd":"/tmp"}
{"type":"custom","customType":"omo-ultrawork:directive","data":{"text":"..."},"id":"c1","parentId":null,"timestamp":"2026-07-29T15:20:00.000Z"}
{"type":"message","id":"c2","parentId":"c1","timestamp":"2026-07-29T15:20:01.000Z","message":{"role":"user","content":"hi"}}
{"type":"thinking_level_change","id":"c3","parentId":"c2","timestamp":"2026-07-29T15:20:02.000Z","thinkingLevel":"high"}"#;
        let file = create_test_file(content);

        // when
        let messages = parse_senpi_file(file.path());

        // then
        assert!(messages.is_empty());
    }

    #[test]
    fn test_parse_senpi_keeps_complete_records_before_truncated_trailing_line() {
        // given: senpi appends while a turn is streaming, so the final line of a
        // live session file can be a half-written JSON object. Already-complete
        // usage above it must still be counted.
        let content = concat!(
            r#"{"type":"session","version":3,"id":"senpi_ses_live","timestamp":"2026-07-29T15:19:53.436Z","cwd":"/tmp"}"#,
            "\n",
            r#"{"type":"message","id":"b2","parentId":null,"timestamp":"2026-07-29T15:20:01.000Z","message":{"role":"assistant","model":"claude-opus-5","provider":"anthropic","usage":{"input":2,"output":3,"cacheRead":0,"cacheWrite":0,"totalTokens":5}}}"#,
            "\n",
            r#"{"type":"message","id":"b3","message":{"role":"assistant""#,
        );
        let file = create_test_file(content);

        // when
        let messages = parse_senpi_file(file.path());

        // then
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "claude-opus-5");
        assert_eq!(messages[0].tokens.input, 2);
        assert_eq!(messages[0].tokens.output, 3);
    }

    #[test]
    fn test_parse_senpi_tolerates_blank_lines_and_crlf_framing() {
        // given
        let content = concat!(
            r#"{"type":"session","version":3,"id":"senpi_ses_framing","timestamp":"2026-07-29T15:19:53.436Z","cwd":"/tmp"}"#,
            "\r\n\r\n",
            r#"{"type":"message","id":"b2","parentId":null,"timestamp":"2026-07-29T15:20:01.000Z","message":{"role":"assistant","model":"claude-opus-5","provider":"anthropic","usage":{"input":7,"output":11,"cacheRead":0,"cacheWrite":0,"totalTokens":18}}}"#,
            "\r\n\n",
        );
        let file = create_test_file(content);

        // when
        let messages = parse_senpi_file(file.path());

        // then
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.input, 7);
        assert_eq!(messages[0].tokens.output, 11);
    }

    #[test]
    fn test_parse_senpi_skips_compaction_and_custom_message_records() {
        // given: a compacted long-running session interleaves compaction and
        // custom_message records between assistant turns.
        let content = r#"{"type":"session","version":3,"id":"senpi_ses_compaction","timestamp":"2026-07-29T15:19:53.436Z","cwd":"/tmp"}
{"type":"message","id":"b1","parentId":null,"timestamp":"2026-07-29T15:20:01.000Z","message":{"role":"assistant","model":"claude-opus-5","provider":"anthropic","usage":{"input":1,"output":1,"cacheRead":0,"cacheWrite":0,"totalTokens":2}}}
{"type":"compaction","id":"c1","parentId":"b1","timestamp":"2026-07-29T15:21:00.000Z","summary":"...","firstKeptEntryId":"b1","tokensBefore":120000}
{"type":"custom_message","id":"c2","parentId":"c1","timestamp":"2026-07-29T15:21:05.000Z","customType":"omo:notice","display":false,"content":"...","details":{}}
{"type":"message","id":"b2","parentId":"c2","timestamp":"2026-07-29T15:22:01.000Z","message":{"role":"assistant","model":"claude-opus-5","provider":"anthropic","usage":{"input":3,"output":4,"cacheRead":0,"cacheWrite":0,"totalTokens":7}}}"#;
        let file = create_test_file(content);

        // when
        let messages = parse_senpi_file(file.path());

        // then
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].tokens.output, 1);
        assert_eq!(messages[1].tokens.output, 4);
    }

    #[test]
    fn test_parse_senpi_skips_usage_without_model_and_keeps_valid_neighbor() {
        // given
        let content = r#"{"type":"session","version":3,"id":"senpi_ses_nomodel","timestamp":"2026-07-29T15:19:53.436Z","cwd":"/tmp"}
{"type":"message","id":"b1","parentId":null,"timestamp":"2026-07-29T15:20:01.000Z","message":{"role":"assistant","provider":"anthropic","usage":{"input":9,"output":9,"cacheRead":0,"cacheWrite":0,"totalTokens":18}}}
{"type":"message","id":"b2","parentId":"b1","timestamp":"2026-07-29T15:20:05.000Z","message":{"role":"assistant","model":"claude-opus-5","provider":"anthropic","usage":{"input":5,"output":6,"cacheRead":0,"cacheWrite":0,"totalTokens":11}}}"#;
        let file = create_test_file(content);

        // when
        let messages = parse_senpi_file(file.path());

        // then
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "claude-opus-5");
        assert_eq!(messages[0].tokens.output, 6);
    }

    #[test]
    fn test_parse_senpi_accepts_forked_session_header_with_parent_session() {
        // given: senpi records the source session id on a forked session header.
        let content = r#"{"type":"session","version":3,"id":"senpi_ses_fork","parentSession":"senpi_ses_origin","timestamp":"2026-07-29T15:19:53.436Z","cwd":"/tmp"}
{"type":"message","id":"b2","parentId":null,"timestamp":"2026-07-29T15:20:01.000Z","message":{"role":"assistant","model":"claude-opus-5","provider":"anthropic","usage":{"input":4,"output":8,"cacheRead":0,"cacheWrite":0,"totalTokens":12}}}"#;
        let file = create_test_file(content);

        // when
        let messages = parse_senpi_file(file.path());

        // then
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].session_id, "senpi_ses_fork");
        assert_eq!(messages[0].tokens.output, 8);
    }

    #[test]
    fn test_parse_senpi_empty_and_header_only_files_are_empty() {
        // given
        let empty = create_test_file("");
        let header_only = create_test_file(
            r#"{"type":"session","version":3,"id":"senpi_ses_empty","timestamp":"2026-07-29T15:19:53.436Z","cwd":"/tmp"}"#,
        );

        // when / then
        assert!(parse_senpi_file(empty.path()).is_empty());
        assert!(parse_senpi_file(header_only.path()).is_empty());
    }

    #[test]
    fn test_parse_senpi_rejects_file_without_session_header() {
        // given
        let content = r#"{"type":"message","id":"b2","parentId":null,"timestamp":"2026-07-29T15:20:01.000Z","message":{"role":"assistant","model":"claude-opus-5","provider":"anthropic","usage":{"input":1,"output":1,"cacheRead":0,"cacheWrite":0,"totalTokens":2}}}"#;
        let file = create_test_file(content);

        // when
        let messages = parse_senpi_file(file.path());

        // then
        assert!(messages.is_empty());
    }
}
