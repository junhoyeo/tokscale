use super::UnifiedMessage;
use crate::TokenBreakdown;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WarpUsageCache {
    synced_at: Option<String>,
    usage: Option<WarpAggregateUsage>,
    #[serde(default)]
    workspaces: Vec<WarpWorkspaceUsage>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WarpAggregateUsage {
    requests_used: Option<i64>,
    spend_cents: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WarpWorkspaceUsage {
    id: Option<String>,
    name: Option<String>,
    requests_used: Option<i64>,
    spend_cents: Option<i64>,
}

pub fn parse_warp_file(path: &Path) -> Vec<UnifiedMessage> {
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(_) => return Vec::new(),
    };
    let cache: WarpUsageCache = match serde_json::from_str(&content) {
        Ok(cache) => cache,
        Err(_) => return Vec::new(),
    };
    let timestamp = cache
        .synced_at
        .as_deref()
        .and_then(parse_rfc3339_millis)
        .unwrap_or(0);
    if timestamp <= 0 {
        return Vec::new();
    }

    let workspace_messages: Vec<UnifiedMessage> = cache
        .workspaces
        .iter()
        .filter_map(|workspace| workspace_to_message(workspace, timestamp))
        .collect();
    if !workspace_messages.is_empty() {
        return workspace_messages;
    }

    cache
        .usage
        .as_ref()
        .and_then(|usage| usage_to_message(usage, timestamp))
        .into_iter()
        .collect()
}

fn usage_to_message(usage: &WarpAggregateUsage, timestamp: i64) -> Option<UnifiedMessage> {
    let requests = non_negative_i32(usage.requests_used);
    let spend_cents = non_negative_i64(usage.spend_cents);
    if requests == 0 && spend_cents == 0 {
        return None;
    }

    let mut message = UnifiedMessage::new(
        "warp",
        "aggregate-requests",
        "warp",
        "warp-aggregate-account",
        timestamp,
        TokenBreakdown::default(),
        cents_to_dollars(spend_cents),
    );
    message.message_count = requests;
    Some(message)
}

fn workspace_to_message(workspace: &WarpWorkspaceUsage, timestamp: i64) -> Option<UnifiedMessage> {
    let requests = non_negative_i32(workspace.requests_used);
    let spend_cents = non_negative_i64(workspace.spend_cents);
    if requests == 0 && spend_cents == 0 {
        return None;
    }

    let workspace_id = workspace
        .id
        .as_deref()
        .map(sanitize_id)
        .filter(|id| !id.is_empty())
        .unwrap_or_else(|| "unknown".to_string());
    let mut message = UnifiedMessage::new(
        "warp",
        "aggregate-requests",
        "warp",
        format!("warp-aggregate-{workspace_id}"),
        timestamp,
        TokenBreakdown::default(),
        cents_to_dollars(spend_cents),
    );
    message.message_count = requests;
    message.set_workspace(
        workspace.id.clone().filter(|id| !id.trim().is_empty()),
        workspace
            .name
            .clone()
            .filter(|name| !name.trim().is_empty()),
    );
    Some(message)
}

fn parse_rfc3339_millis(value: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|dt| dt.with_timezone(&Utc).timestamp_millis())
}

fn non_negative_i64(value: Option<i64>) -> i64 {
    value.unwrap_or(0).max(0)
}

fn non_negative_i32(value: Option<i64>) -> i32 {
    non_negative_i64(value).min(i32::MAX as i64) as i32
}

fn cents_to_dollars(cents: i64) -> f64 {
    cents as f64 / 100.0
}

fn sanitize_id(value: &str) -> String {
    value
        .trim()
        .to_lowercase()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.' {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}
