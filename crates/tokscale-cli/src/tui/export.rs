use anyhow::Result;
use serde_json::json;

use super::data::UsageData;

/// Serializes `UsageData` into the pretty-printed JSON payload used by the
/// `e` export hotkey. Pure: callers are responsible for file I/O and any
/// user-facing status messages.
pub fn build_export_json(data: &UsageData) -> Result<String> {
    let export_data = json!({
        "models": data.models.iter().map(|m| json!({
            "model": m.model,
            "provider": m.provider,
            "client": m.client,
            "tokens": {
                "input": m.tokens.input,
                "output": m.tokens.output,
                "cacheRead": m.tokens.cache_read,
                "cacheWrite": m.tokens.cache_write,
                "total": m.tokens.total()
            },
            "cost": m.cost,
            "sessionCount": m.session_count
        })).collect::<Vec<_>>(),
        "agents": data.agents.iter().map(|a| json!({
            "agent": a.agent,
            "clients": a.clients,
            "tokens": {
                "input": a.tokens.input,
                "output": a.tokens.output,
                "cacheRead": a.tokens.cache_read,
                "cacheWrite": a.tokens.cache_write,
                "total": a.tokens.total()
            },
            "cost": a.cost,
            "messageCount": a.message_count
        })).collect::<Vec<_>>(),
        "daily": data.daily.iter().map(|d| json!({
            "date": d.date.to_string(),
            "tokens": {
                "input": d.tokens.input,
                "output": d.tokens.output,
                "cacheRead": d.tokens.cache_read,
                "cacheWrite": d.tokens.cache_write,
                "total": d.tokens.total()
            },
            "messageCount": d.message_count,
            "turnCount": d.turn_count,
            "cost": d.cost
        })).collect::<Vec<_>>(),
        "totals": {
            "tokens": data.total_tokens,
            "cost": data.total_cost
        }
    });

    Ok(serde_json::to_string_pretty(&export_data)?)
}
