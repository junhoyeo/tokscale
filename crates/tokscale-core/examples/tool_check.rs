//! Cross-checks the parser's tool-call extraction against the raw transcript.
//!
//! Run against real Claude Code transcripts to confirm the streaming-frame
//! merge reconstructs every call:
//!
//! ```text
//! cargo run --example tool_check -p tokscale-core -- ~/.claude/projects/**/*.jsonl
//! ```
fn main() {
    let mut mismatches = 0usize;
    let mut files = 0usize;
    let mut total = 0u64;
    for path in std::env::args().skip(1) {
        let p = std::path::Path::new(&path);
        let msgs = tokscale_core::sessions::claudecode::parse_claude_file(p);
        let parsed: u64 = msgs
            .iter()
            .filter_map(|m| m.tool_calls.as_ref())
            .flatten()
            .map(|c| c.count as u64)
            .sum();

        let raw = std::fs::read_to_string(p).unwrap_or_default();
        let mut truth = 0u64;
        for line in raw.lines() {
            let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            if v.get("type").and_then(|t| t.as_str()) != Some("assistant") {
                continue;
            }
            let Some(blocks) = v.pointer("/message/content").and_then(|c| c.as_array()) else {
                continue;
            };
            truth += blocks
                .iter()
                .filter(|b| {
                    matches!(
                        b.get("type").and_then(|t| t.as_str()),
                        Some("tool_use") | Some("server_tool_use")
                    )
                })
                .count() as u64;
        }

        files += 1;
        total += truth;
        if parsed != truth {
            mismatches += 1;
            if mismatches <= 5 {
                println!("MISMATCH parsed={parsed} truth={truth}  {}", p.display());
            }
        }
    }
    println!("files={files} tool_calls={total} mismatches={mismatches}");
}
