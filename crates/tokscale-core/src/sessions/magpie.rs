//! Attribute Codex's response-scoped usage to the model reported by Magpie.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;

use serde::Deserialize;

use super::opencodex::{read_bounded_line, BoundedLine};
use super::{CostSource, UnifiedMessage};

#[derive(Deserialize)]
struct LedgerRow {
    agent: Option<String>,
    response_id: Option<String>,
    provider: Option<String>,
    served: Option<String>,
    req: Option<String>,
    #[serde(rename = "in")]
    input: Option<i64>,
    out: Option<i64>,
    cache_read: Option<i64>,
    cache_write: Option<i64>,
    reasoning: Option<i64>,
    status: Option<i32>,
    err: Option<String>,
    source: Option<String>,
    computer: Option<String>,
    via: Option<String>,
    rejected: Option<bool>,
    handed_on: Option<bool>,
    tier_served: Option<String>,
}

fn response_id(message: &UnifiedMessage) -> Option<&str> {
    if message.client != "codex"
        || message.message_count != 1
        || message.model_attribution_conflicted
    {
        return None;
    }
    let key = message
        .dedup_key
        .as_deref()?
        .strip_prefix("codex:usage-record:")?;
    let (scope, id) = key.split_once(':')?;
    (!scope.is_empty() && !id.is_empty() && !id.contains(':')).then_some(id)
}

fn ledger_path(home: &str, use_env_roots: bool) -> PathBuf {
    if use_env_roots {
        if let Some(root) = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from) {
            if root.is_absolute() {
                return root.join("magpie/usage.jsonl");
            }
        }
    }
    PathBuf::from(home).join(".config/magpie/usage.jsonl")
}

/// Use a unique, completed local gateway response only when its counters agree
/// with the Codex response record. The ledger never supplies additional usage.
pub(crate) fn attribute_codex_responses(
    home: &str,
    use_env_roots: bool,
    messages: &mut [UnifiedMessage],
    already_attributed: &HashSet<usize>,
) -> usize {
    let mut wanted = HashMap::new();
    for (index, message) in messages.iter().enumerate() {
        if already_attributed.contains(&index) {
            continue;
        }
        if let Some(id) = response_id(message) {
            wanted
                .entry(id.to_string())
                .and_modify(|slot| *slot = None)
                .or_insert(Some(index));
        }
    }
    if wanted.is_empty() {
        return 0;
    }

    let Ok(file) = File::open(ledger_path(home, use_env_roots)) else {
        return 0;
    };
    let mut reader = BufReader::new(file);
    let mut line = Vec::new();
    let mut rows: HashMap<String, Option<LedgerRow>> = HashMap::new();
    loop {
        match read_bounded_line(&mut reader, &mut line) {
            Ok(BoundedLine::Eof) => break,
            Ok(BoundedLine::Oversized) => continue,
            Ok(BoundedLine::Line) => {}
            Err(_) => return 0,
        }
        let Ok(row) = simd_json::serde::from_slice::<LedgerRow>(&mut line) else {
            continue;
        };
        let Some(id) = row.response_id.as_deref() else {
            continue;
        };
        if !wanted.contains_key(id) {
            continue;
        }
        rows.entry(id.to_string())
            .and_modify(|existing| *existing = None)
            .or_insert(Some(row));
    }

    let mut attributed = 0;
    for (id, index) in wanted {
        let (Some(index), Some(Some(row))) = (index, rows.remove(&id)) else {
            continue;
        };
        let message = &mut messages[index];
        let (Some(provider), Some(served), Some(input), Some(output)) = (
            row.provider.as_deref(),
            row.served.as_deref(),
            row.input,
            row.out,
        ) else {
            continue;
        };
        let served = served.trim();
        let lower = served.to_ascii_lowercase();
        let gateway_input = input
            .checked_add(row.cache_read.unwrap_or(0))
            .and_then(|n| n.checked_add(row.cache_write.unwrap_or(0)));
        let recorded_input = message
            .tokens
            .input
            .checked_add(message.tokens.cache_read)
            .and_then(|n| n.checked_add(message.tokens.cache_write));
        let recorded_output = message.tokens.output.checked_add(message.tokens.reasoning);
        if row.agent.as_deref() != Some("codex")
            || !row
                .status
                .is_some_and(|status| (200..300).contains(&status))
            || row.err.as_deref().is_some_and(|err| !err.is_empty())
            || row.source.as_deref() == Some("reported")
            || row
                .computer
                .as_deref()
                .is_some_and(|computer| !computer.is_empty())
            || row.via.as_deref().is_some_and(|via| !via.is_empty())
            || row.rejected == Some(true)
            || row.handed_on == Some(true)
            || provider.trim().is_empty()
            || served.is_empty()
            || lower == "auto"
            || lower == "agent_review"
            || crate::pricing::lookup::is_routing_label(served)
            || lower.starts_with("group/")
            || lower.contains("/group/")
            || lower.ends_with("/auto")
            || row
                .req
                .as_deref()
                .is_some_and(|req| req.eq_ignore_ascii_case(served))
            || input < 0
            || output < 0
            || row.cache_read.unwrap_or(0) < 0
            || row.cache_write.unwrap_or(0) < 0
            || row
                .cache_read
                .is_some_and(|tokens| tokens != message.tokens.cache_read)
            || row
                .cache_write
                .is_some_and(|tokens| tokens != message.tokens.cache_write)
            || row
                .reasoning
                .is_some_and(|tokens| tokens != message.tokens.reasoning)
            || gateway_input.is_none()
            || gateway_input != recorded_input
            || Some(output) != recorded_output
        {
            continue;
        }
        let changed_model = message.model_id != served;
        let changed_tier = row
            .tier_served
            .as_ref()
            .is_some_and(|tier| message.service_tier.as_ref() != Some(tier));
        if !changed_model && message.provider_id == provider && !changed_tier {
            continue;
        }
        message.model_id = served.to_string();
        message.provider_id = provider.to_string();
        message.pricing_model_id = None;
        if changed_model || row.tier_served.is_some() {
            message.service_tier = row.tier_served;
        }
        if !message.has_authoritative_cost() {
            message.cost = 0.0;
            message.cost_source = CostSource::Unknown;
        }
        attributed += 1;
    }
    attributed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TokenBreakdown;

    #[test]
    fn fallback_usage_key_does_not_supply_a_response_id() {
        let mut message = UnifiedMessage::new(
            "codex",
            "group/auto",
            "openai",
            "session",
            1,
            TokenBreakdown::default(),
            0.0,
        );
        message.dedup_key = Some("codex:usage-record:session:123:500:40:200:10".into());
        assert_eq!(response_id(&message), None);
        message.dedup_key = Some("codex:usage-record:session:resp-123".into());
        assert_eq!(response_id(&message), Some("resp-123"));
    }
}
