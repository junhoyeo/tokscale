//! Magpie gateway adapter.
//!
//! Magpie records every call it serves in `~/.config/magpie/usage.jsonl`.
//! A record names the model the agent asked for (`req`), the model the
//! gateway's chosen provider was contacted with (`model`), and the model the
//! vendor's reply said answered (`served`), plus the provider, session
//! headers, duration, and the token counters of the answering attempt.
//!
//! Two record kinds are excluded on purpose:
//!
//! - rows the gateway refused locally never reached an upstream;
//! - rows with `handed_on` are failed tries that were retried elsewhere, so
//!   they are not the request's outcome;
//! - rows with `source: "reported"` are calls a client backfilled after the
//!   fact, not calls this gateway served;
//! - rows with a `computer` id were synced from another machine, so they
//!   cannot describe a local client's request.
//!
//! A record's own `served` value is preferred evidence. `model` is a
//! confirmed gateway selection and is admissible only as a fallback, because a
//! routing group chooses a member that the vendor may still redirect.
//!
//! The ledger is evidence only: attributing a record to a client row never
//! counts the record as usage, so a call through two gateways cannot double
//! count.

use std::path::PathBuf;
use std::sync::Arc;

use super::matcher::non_empty;
use super::matcher::{GatewayIndex, GatewayRecord, MatchPolicy, TokenSignature};
use super::{cached_index, read_ledger};

/// Where Magpie keeps its ledger. Installed builds use the XDG config
/// directory (`~/.config/magpie` on every platform); a portable build keeps it
/// beside the executable, which a scan cannot discover from the home
/// directory, so those users stay unattributed rather than guessed.
fn ledger_path(home_dir: &str, use_env_roots: bool) -> PathBuf {
    if use_env_roots {
        if let Some(raw) = std::env::var_os("XDG_CONFIG_HOME").filter(|value| !value.is_empty()) {
            return PathBuf::from(raw).join("magpie").join("usage.jsonl");
        }
    }
    PathBuf::from(home_dir)
        .join(".config")
        .join("magpie")
        .join("usage.jsonl")
}

#[derive(Debug, Clone, serde::Deserialize)]
struct LedgerRow {
    #[serde(default)]
    t: Option<String>,
    #[serde(default)]
    provider: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    req: Option<String>,
    #[serde(default)]
    served: Option<String>,
    #[serde(rename = "in", default)]
    input: Option<i64>,
    #[serde(default)]
    out: Option<i64>,
    #[serde(default)]
    cache_read: Option<i64>,
    #[serde(default)]
    cache_write: Option<i64>,
    #[serde(default)]
    reasoning: Option<i64>,
    #[serde(default)]
    ms: Option<i64>,
    #[serde(default)]
    status: Option<i32>,
    #[serde(default)]
    tier_served: Option<String>,
    #[serde(default)]
    rid: Option<String>,
    #[serde(default)]
    response_id: Option<String>,
    #[serde(default)]
    session: Option<String>,
    #[serde(default)]
    native_session: Option<String>,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    computer: Option<String>,
    #[serde(default)]
    rejected: Option<bool>,
    #[serde(default)]
    handed_on: Option<bool>,
}

/// Decode one ledger row into normalized evidence, or nothing when the row
/// cannot describe a completed gateway-served call.
fn decode_row(line: &[u8]) -> Option<GatewayRecord> {
    let row: LedgerRow = serde_json::from_slice(line).ok()?;

    if row.rejected == Some(true) || row.handed_on == Some(true) {
        return None;
    }
    if row.source.as_deref() == Some("reported") {
        return None;
    }
    if non_empty(row.computer.as_deref()).is_some() {
        return None;
    }
    if !row
        .status
        .is_some_and(|status| (200..300).contains(&status))
    {
        return None;
    }
    let provider = non_empty(row.provider.as_deref())?.to_string();
    let timestamp = row
        .t
        .as_deref()
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.timestamp_millis())
        .filter(|value| *value > 0)?;
    let duration_ms = row.ms.filter(|duration| *duration >= 0);

    // A signature needs both additive totals. Older ledger rows that predate
    // them can still match on a stable identifier, but never on tokens; the
    // per-bucket counters are passed through as reported, so a row that omits
    // one does not contradict a client that reports it.
    let signatures = match (row.input, row.out) {
        (Some(input), Some(output)) if input >= 0 && output >= 0 => {
            vec![TokenSignature::from_gateway(
                input,
                output,
                row.cache_read.filter(|value| *value >= 0),
                row.cache_write.filter(|value| *value >= 0),
                row.reasoning.filter(|value| *value >= 0),
            )]
        }
        _ => Vec::new(),
    };

    let session_ids: Vec<String> = [row.native_session.as_deref(), row.session.as_deref()]
        .into_iter()
        .filter_map(non_empty)
        .map(str::to_string)
        .collect();
    // `response_id` is the response object id the client received; `rid` is
    // the vendor's request id. Both are stable identifiers a client parser can
    // repeat in its dedup key, so either can establish that a record and a
    // message describe one call.
    let stable_ids: Vec<String> = [row.response_id.as_deref(), row.rid.as_deref()]
        .into_iter()
        .filter_map(non_empty)
        .map(str::to_string)
        .collect();

    Some(GatewayRecord {
        adapter: "magpie",
        selector: non_empty(row.req.as_deref()).map(str::to_string),
        selected_model: non_empty(row.model.as_deref()).map(str::to_string),
        served_model: non_empty(row.served.as_deref()).map(str::to_string),
        provider: Some(provider),
        service_tier: non_empty(row.tier_served.as_deref()).map(str::to_string),
        session_ids,
        stable_ids,
        start_ms: timestamp,
        duration_ms,
        signatures,
        completed: true,
        target_eligible: true,
        allow_selected_fallback: true,
    })
}

/// Load the gateway index for this machine, honoring `XDG_CONFIG_HOME` when
/// env-derived roots are enabled. Returns `None` when Magpie has no ledger.
pub(crate) fn load_index(home_dir: &str, use_env_roots: bool) -> Option<Arc<GatewayIndex>> {
    let path = ledger_path(home_dir, use_env_roots);
    cached_index(&path, MatchPolicy::magpie(), |path| {
        read_ledger(path, decode_row).ok()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{TokenBreakdown, UnifiedMessage};
    use serde_json::{json, Value};

    const START: i64 = 1_760_000_000_000;
    const SELECTOR: &str = "group/auto";

    fn row() -> Value {
        json!({
            "t": "2025-10-09T08:53:20.000Z",
            "provider": "provider-a",
            "model": "provider-a/wire-model",
            "req": SELECTOR,
            "served": "provider-a/served-model",
            "in": 500,
            "out": 25,
            "cache_read": 400,
            "cache_write": 0,
            "reasoning": 5,
            "ms": 8000,
            "status": 200,
            "session": "session-1"
        })
    }

    fn write(home: &std::path::Path, rows: &[Value]) {
        let dir = home.join(".config/magpie");
        std::fs::create_dir_all(&dir).unwrap();
        let mut body = String::new();
        for row in rows {
            body.push_str(&serde_json::to_string(row).unwrap());
            body.push('\n');
        }
        std::fs::write(dir.join("usage.jsonl"), body).unwrap();
    }

    fn message() -> UnifiedMessage {
        let mut message = UnifiedMessage::new(
            "claude",
            SELECTOR,
            "anthropic",
            "session-1",
            START,
            TokenBreakdown {
                input: 500,
                output: 20,
                cache_read: 400,
                cache_write: 0,
                reasoning: 5,
                ..Default::default()
            },
            0.0,
        );
        message.duration_ms = Some(8_000);
        message
    }

    fn index_for(home: &std::path::Path) -> Arc<GatewayIndex> {
        load_index(home.to_str().unwrap(), false).expect("the ledger must load")
    }

    #[test]
    fn decodes_a_completed_row_into_served_evidence() {
        let home = tempfile::TempDir::new().unwrap();
        write(home.path(), &[row()]);
        let index = index_for(home.path());

        assert_eq!(index.len(), 1);
        let candidates = index.candidates(&message());
        assert_eq!(candidates.len(), 1);
        let target = index.record(candidates[0].record).target().unwrap();
        assert_eq!(target.model, "provider-a/served-model");
        assert_eq!(target.provider, "provider-a");
    }

    #[test]
    fn a_row_without_served_uses_the_confirmed_selection() {
        let home = tempfile::TempDir::new().unwrap();
        let mut value = row();
        value.as_object_mut().unwrap().remove("served");
        write(home.path(), &[value]);
        let index = index_for(home.path());

        let candidates = index.candidates(&message());
        assert_eq!(candidates.len(), 1);
        let target = index.record(candidates[0].record).target().unwrap();
        assert_eq!(target.model, "provider-a/wire-model");
    }

    #[test]
    fn failed_handed_on_reported_and_foreign_rows_are_ignored() {
        for mutate in [
            |value: &mut Value| value["status"] = json!(429),
            |value: &mut Value| value["handed_on"] = json!(true),
            |value: &mut Value| value["source"] = json!("reported"),
            |value: &mut Value| value["computer"] = json!("laptop-2"),
            |value: &mut Value| value["rejected"] = json!(true),
        ] {
            let home = tempfile::TempDir::new().unwrap();
            let mut value = row();
            mutate(&mut value);
            write(home.path(), &[value]);
            assert!(
                load_index(home.path().to_str().unwrap(), false)
                    .is_none_or(|index| index.is_empty()),
                "row must not become evidence"
            );
        }
    }

    /// A router that answers `served` with the selector it was asked for has
    /// named no physical model. The record must fall back to the confirmed
    /// selection instead of promoting the routing label to a model identity.
    #[test]
    fn an_echoed_selector_is_not_response_evidence() {
        let home = tempfile::TempDir::new().unwrap();
        let mut value = row();
        value["served"] = json!(SELECTOR);
        write(home.path(), &[value]);
        let index = index_for(home.path());

        let candidates = index.candidates(&message());
        assert_eq!(candidates.len(), 1);
        let target = index.record(candidates[0].record).target().unwrap();
        assert_eq!(target.model, "provider-a/wire-model");
        assert_eq!(
            target.strength,
            crate::model_attribution::matcher::EvidenceStrength::ConfirmedSelection
        );
    }

    #[test]
    fn a_token_mismatch_does_not_match() {
        let home = tempfile::TempDir::new().unwrap();
        let mut value = row();
        value["out"] = json!(26);
        write(home.path(), &[value]);
        let index = index_for(home.path());

        assert!(index.candidates(&message()).is_empty());
    }

    #[test]
    fn an_older_row_without_totals_matches_only_by_identifier() {
        let home = tempfile::TempDir::new().unwrap();
        let mut value = row();
        value.as_object_mut().unwrap().remove("in");
        value.as_object_mut().unwrap().remove("out");
        write(home.path(), &[value]);
        let index = index_for(home.path());

        assert!(index.candidates(&message()).is_empty());

        let mut message = message();
        message.dedup_key = Some("pi:response:resp-magpie".into());
        let index = index_with_response_id(home.path(), "resp-magpie");
        let candidates = index.candidates(&message);
        assert_eq!(candidates.len(), 1);
    }

    fn index_with_response_id(home: &std::path::Path, id: &str) -> Arc<GatewayIndex> {
        let mut value = row();
        value["response_id"] = json!(id);
        value.as_object_mut().unwrap().remove("in");
        value.as_object_mut().unwrap().remove("out");
        value.as_object_mut().unwrap().remove("session");
        write(home, &[value]);
        super::super::clear_index_cache();
        index_for(home)
    }

    #[test]
    fn an_appended_row_invalidates_the_cached_index() {
        let home = tempfile::TempDir::new().unwrap();
        write(home.path(), &[row()]);
        assert_eq!(index_for(home.path()).len(), 1);

        let mut second = row();
        second["session"] = json!("session-2");
        write(home.path(), &[row(), second]);
        assert_eq!(index_for(home.path()).len(), 2);
    }

    #[test]
    fn the_service_tier_comes_only_from_the_response() {
        let home = tempfile::TempDir::new().unwrap();
        let mut value = row();
        value["tier_served"] = json!("priority");
        write(home.path(), &[value]);
        let index = index_for(home.path());

        let target = index.record(0).target().unwrap();
        assert_eq!(target.service_tier.as_deref(), Some("priority"));
    }
}
