//! Aggregates tool calls the way the TUI's Tools tab does, over a real scan.
//!
//! ```text
//! cargo run --example tool_totals -p tokscale-core
//! ```
use std::collections::BTreeMap;

#[tokio::main]
async fn main() -> Result<(), String> {
    let home = dirs::home_dir().ok_or("no home directory")?;
    let options = tokscale_core::LocalParseOptions {
        home_dir: Some(home.to_string_lossy().into_owned()),
        use_env_roots: true,
        clients: None,
        since: None,
        until: None,
        year: None,
        scanner_settings: Default::default(),
    };
    let messages = tokscale_core::parse_local_unified_messages(options).await?;

    let mut by_tool: BTreeMap<(String, bool), (u64, u32)> = BTreeMap::new();
    let mut unknown = 0u64;
    let mut known = 0u64;
    for msg in &messages {
        match &msg.tool_calls {
            Some(calls) => {
                known += 1;
                for call in calls {
                    let entry = by_tool.entry((call.name.clone(), call.mcp)).or_default();
                    entry.0 += u64::from(call.count);
                    entry.1 += 1;
                }
            }
            None => unknown += 1,
        }
    }

    let total: u64 = by_tool.values().map(|(calls, _)| calls).sum();
    println!(
        "messages={} known={known} unknown={unknown}",
        messages.len()
    );
    println!("distinct tools={} total calls={total}", by_tool.len());

    let mut rows: Vec<_> = by_tool.into_iter().collect();
    rows.sort_by(|a, b| b.1 .0.cmp(&a.1 .0));
    for ((name, mcp), (calls, msgs)) in rows.into_iter().take(15) {
        let kind = if mcp { "MCP" } else { "built-in" };
        println!("  {name:<26} {calls:>8} calls  {msgs:>7} msgs  {kind}");
    }
    Ok(())
}
