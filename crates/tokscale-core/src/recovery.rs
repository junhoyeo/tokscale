//! Local, provenance-labelled disaster-recovery overlay. Never used by submission.
//! Raw messages win. Request archives supply absent sessions, and aggregate
//! snapshots supply only positive gaps over the resulting daily totals.
use crate::{CostSource, TokenBreakdown, UnifiedMessage};
use serde::Deserialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Deserialize)]
struct Archive {
    version: u32,
    home: String,
    #[serde(default)]
    messages: Vec<UnifiedMessage>,
    #[serde(default)]
    daily_floors: Vec<UnifiedMessage>,
}

pub fn path() -> PathBuf {
    crate::paths::get_config_dir().join("recovered-usage-v1.json")
}
pub fn enabled() -> bool {
    std::env::var_os("TOKSCALE_RECOVERY_DISABLE").is_none() && path().is_file()
}
fn key(m: &UnifiedMessage) -> (String, String, String) {
    (
        m.client.clone(),
        m.date.clone(),
        crate::model_name_for_grouping(&m.client, &m.provider_id, &m.model_id),
    )
}
fn total(t: &TokenBreakdown) -> i64 {
    t.input + t.output + t.cache_read + t.cache_write + t.reasoning
}
pub fn is_daily(m: &UnifiedMessage) -> bool {
    m.dedup_key
        .as_deref()
        .unwrap_or("")
        .starts_with("local-recovery:daily:")
}
fn finite(m: &UnifiedMessage) -> bool {
    [
        m.tokens.input,
        m.tokens.output,
        m.tokens.cache_read,
        m.tokens.cache_write,
        m.tokens.reasoning,
    ]
    .iter()
    .all(|v| *v >= 0 && *v < 1_000_000_000_000_000)
        && m.cost.is_finite()
        && m.cost >= 0.0
        && m.message_count >= 0
        && chrono::NaiveDate::parse_from_str(&m.date, "%Y-%m-%d").is_ok()
}
fn signature(m: &UnifiedMessage) -> (String, String, String, i64, i64, i64, i64, i64, i64) {
    (
        m.client.clone(),
        m.session_id.clone(),
        crate::model_name_for_grouping(&m.client, &m.provider_id, &m.model_id),
        m.timestamp / 1000,
        m.tokens.input,
        m.tokens.output,
        m.tokens.reasoning,
        m.tokens.cache_read,
        m.tokens.cache_write,
    )
}

/// True when the ledger header is version 1 for this home.
///
/// The check reads only the prefix before `messages`, so a foreign or corrupt
/// file does not force the local graph off the streaming path. `version` and
/// `home` have to appear within the first 64KiB.
pub fn applicable(home: &str) -> bool {
    if !enabled() {
        return false;
    }
    let Ok(file) = std::fs::File::open(path()) else {
        return false;
    };
    let mut buf = vec![0u8; 65_536];
    let Ok(n) = std::io::Read::read(&mut std::io::BufReader::new(file), &mut buf) else {
        return false;
    };
    header_matches(&buf[..n], home)
}

fn header_matches(bytes: &[u8], home: &str) -> bool {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return false;
    };
    let Some(messages_at) = text.find("\"messages\"") else {
        return false;
    };
    let mut prefix = text[..messages_at]
        .trim_end()
        .trim_end_matches(',')
        .to_string();
    prefix.push('}');
    #[derive(Deserialize)]
    struct Header {
        version: u32,
        home: String,
    }
    let Ok(header) = serde_json::from_str::<Header>(&prefix) else {
        return false;
    };
    header.version == 1 && Path::new(&header.home) == Path::new(home)
}

fn align_dates(rows: &mut [UnifiedMessage], timezone: Option<&crate::bucket_tz::BucketTimezone>) {
    let Some(timezone) = timezone.filter(|timezone| timezone.is_pinned()) else {
        return;
    };
    for row in rows {
        row.rebucket_date(timezone);
    }
}

pub fn apply(
    messages: &mut Vec<UnifiedMessage>,
    home: &str,
    clients: &[String],
    timezone: Option<&crate::bucket_tz::BucketTimezone>,
) {
    if !enabled() {
        return;
    }
    let result = (|| -> Result<Archive, Box<dyn std::error::Error>> {
        let f = std::fs::File::open(path())?;
        Ok(serde_json::from_reader(std::io::BufReader::new(f))?)
    })();
    let mut archive = match result {
        Ok(a) if a.version == 1 && Path::new(&a.home) == Path::new(home) => a,
        _ => return,
    };
    align_dates(&mut archive.messages, timezone);
    align_dates(&mut archive.daily_floors, timezone);
    let all = clients.is_empty();
    let requested: HashSet<&str> = clients.iter().map(String::as_str).collect();
    merge(messages, archive.messages, archive.daily_floors, |m| {
        all || crate::retain_for_requested_clients(
            &m.client,
            &m.model_id,
            &m.provider_id,
            &requested,
        )
    });
}

pub fn merge<F: Fn(&UnifiedMessage) -> bool>(
    native: &mut Vec<UnifiedMessage>,
    recovered: Vec<UnifiedMessage>,
    floors: Vec<UnifiedMessage>,
    include: F,
) {
    // Reapplying the overlay is idempotent, including on an already merged vector.
    native.retain(|m| {
        !m.dedup_key
            .as_deref()
            .unwrap_or("")
            .starts_with("local-recovery:")
    });
    let sessions: HashSet<(String, String)> = native
        .iter()
        .map(|m| (m.client.clone(), m.session_id.clone()))
        .collect();
    let mut seen: HashSet<_> = native.iter().map(signature).collect();
    let mut ids: HashSet<String> = HashSet::new();
    // If any original transcript for the session is present, its request rows
    // are not appended blindly. The exporter reconciles partial sessions into
    // daily floors instead. This prevents fork/replay and timestamp collisions.
    for mut m in recovered {
        if !include(&m)
            || !finite(&m)
            || sessions.contains(&(m.client.clone(), m.session_id.clone()))
        {
            continue;
        }
        let id = m.dedup_key.clone().unwrap_or_default();
        if id.is_empty() || !ids.insert(id.clone()) || !seen.insert(signature(&m)) {
            continue;
        }
        m.dedup_key = Some(format!("local-recovery:{id}"));
        m.agent = Some("Recovered request archive".into());
        m.is_turn_start = false;
        native.push(m);
    }
    let mut sums: std::collections::BTreeMap<_, (i64, f64, i64)> =
        std::collections::BTreeMap::new();
    for m in native.iter() {
        let x = sums.entry(key(m)).or_default();
        x.0 += total(&m.tokens);
        x.1 += m.cost;
        x.2 += m.message_count as i64;
    }
    // Reconcile the WHOLE client/day first. Per-model maxima across tools
    // can double count a request that was renamed/reclassified between snapshots.
    let mut unique: std::collections::BTreeMap<_, UnifiedMessage> =
        std::collections::BTreeMap::new();
    for m in floors {
        if !include(&m) || !finite(&m) {
            continue;
        }
        let k = key(&m);
        if unique
            .get(&k)
            .is_none_or(|old| total(&m.tokens) > total(&old.tokens))
        {
            unique.insert(k, m);
        }
    }
    let mut groups: std::collections::BTreeMap<(String, String), Vec<UnifiedMessage>> =
        std::collections::BTreeMap::new();
    for (k, m) in unique {
        groups.entry((k.0, k.1)).or_default().push(m);
    }
    for (day, group) in groups {
        let target: i64 = group.iter().map(|m| total(&m.tokens)).sum();
        let target_cost: f64 = group.iter().map(|m| m.cost).sum();
        let target_count: i64 = group.iter().map(|m| m.message_count as i64).sum();
        let current: i64 = sums
            .iter()
            .filter(|(k, _)| k.0 == day.0 && k.1 == day.1)
            .map(|(_, v)| v.0)
            .sum();
        let current_cost: f64 = sums
            .iter()
            .filter(|(k, _)| k.0 == day.0 && k.1 == day.1)
            .map(|(_, v)| v.1)
            .sum();
        let current_count: i64 = sums
            .iter()
            .filter(|(k, _)| k.0 == day.0 && k.1 == day.1)
            .map(|(_, v)| v.2)
            .sum();
        let gap = (target - current).max(0);
        if gap == 0 {
            continue;
        }
        let weights: Vec<i64> = group
            .iter()
            .map(|m| (total(&m.tokens) - sums.get(&key(m)).map(|v| v.0).unwrap_or(0)).max(0))
            .collect();
        let mut remaining_weight: i64 = weights.iter().sum();
        let mut remaining = gap;
        let mut remaining_count = (target_count - current_count).max(0);
        let mut remaining_cost = (target_cost - current_cost).max(0.0);
        for (mut m, weight) in group.into_iter().zip(weights) {
            if weight == 0 {
                continue;
            }
            let part = if weight == remaining_weight {
                remaining
            } else {
                ((remaining as i128 * weight as i128) / remaining_weight as i128) as i64
            };
            remaining_weight -= weight;
            if part == 0 {
                continue;
            }
            let ratio = part as f64 / remaining as f64;
            let count = if part == remaining {
                remaining_count
            } else {
                (remaining_count as f64 * ratio).floor() as i64
            };
            let cost = remaining_cost * ratio;
            remaining -= part;
            remaining_count -= count;
            remaining_cost -= cost;
            let scale = part as f64 / total(&m.tokens) as f64;
            let mut t = TokenBreakdown::default();
            t.output = (m.tokens.output as f64 * scale).floor() as i64;
            t.cache_read = (m.tokens.cache_read as f64 * scale).floor() as i64;
            t.cache_write = (m.tokens.cache_write as f64 * scale).floor() as i64;
            t.reasoning = (m.tokens.reasoning as f64 * scale).floor() as i64;
            t.input = part - t.output - t.cache_read - t.cache_write - t.reasoning;
            m.tokens = t;
            m.cost = cost;
            m.cost_source = CostSource::Estimated;
            m.message_count = count.min(i32::MAX as i64) as i32;
            m.agent = Some("Recovered daily aggregate (component split estimated)".into());
            let k = key(&m);
            m.session_id = format!("local-recovery:daily:{}:{}:{}", k.0, k.1, k.2);
            m.session_title = Some("Recovered daily usage; original sessions unavailable".into());
            m.dedup_key = Some(m.session_id.clone());
            m.is_turn_start = false;
            m.duration_ms = None;
            m.timestamp = 0;
            native.push(m);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn row(id: &str, n: i64) -> UnifiedMessage {
        serde_json::from_value(serde_json::json!({"client":"codex","model_id":"gpt-5.4","provider_id":"openai","session_id":id,"workspace_key":null,"workspace_label":null,"timestamp":1770000000000i64,"date":"2026-02-02","tokens":{"input":n,"output":0,"cache_read":0,"cache_write":0,"reasoning":0},"cost":1.0,"agent":null,"dedup_key":id})).unwrap()
    }
    #[test]
    fn overlap_and_idempotence() {
        let mut n = vec![row("live", 40)];
        let r = vec![row("live", 40), row("missing", 20), row("missing", 20)];
        let f = vec![row("floor", 100), row("floor", 90)];
        merge(&mut n, r.clone(), f.clone(), |_| true);
        assert_eq!(n.iter().map(|m| total(&m.tokens)).sum::<i64>(), 100);
        merge(&mut n, r, f, |_| true);
        assert_eq!(n.iter().map(|m| total(&m.tokens)).sum::<i64>(), 100);
    }
    #[test]
    fn restored_original_supersedes_archive() {
        let mut n = vec![row("missing", 150)];
        merge(
            &mut n,
            vec![row("missing", 100)],
            vec![row("floor", 120)],
            |_| true,
        );
        assert_eq!(n.len(), 1);
        assert_eq!(total(&n[0].tokens), 150);
    }
    #[test]
    fn client_filter_and_invalid() {
        let mut n = vec![];
        merge(&mut n, vec![row("bad", -2), row("ok", 20)], vec![], |_| {
            false
        });
        assert!(n.is_empty());
    }
    #[test]
    fn invalid_calendar_date_is_dropped() {
        let mut bad = row("bad-date", 20);
        bad.date = "2026-99-99".into();
        let mut n = vec![];
        merge(&mut n, vec![bad], vec![], |_| true);
        assert!(n.is_empty());
    }
    #[test]
    fn same_second_different_model_or_reasoning_both_restore() {
        let mut output = row("out", 0);
        output.tokens.output = 20;
        let mut reasoning = row("reason", 0);
        reasoning.session_id = output.session_id.clone();
        reasoning.timestamp = output.timestamp;
        reasoning.tokens.reasoning = 20;
        let mut other_model = row("model", 0);
        other_model.session_id = output.session_id.clone();
        other_model.timestamp = output.timestamp;
        other_model.model_id = "gpt-5.5".into();
        other_model.tokens.output = 20;
        let mut n = vec![];
        merge(&mut n, vec![output, reasoning, other_model], vec![], |_| {
            true
        });
        assert_eq!(n.len(), 3);
    }
    #[test]
    fn pinned_zone_moves_recovered_request_before_floor_gap() {
        let ts = chrono::DateTime::parse_from_rfc3339("2026-02-02T00:30:00Z")
            .unwrap()
            .timestamp_millis();
        let mut recovered = row("missing", 10);
        recovered.timestamp = ts;
        recovered.date = "2026-02-02".into();
        let mut floor = row("floor", 10);
        floor.timestamp = 0;
        floor.date = "2026-02-02".into();
        let timezone =
            crate::bucket_tz::BucketTimezone::from_pinned_name(Some("America/Los_Angeles"));
        let mut messages = vec![recovered];
        let mut floors = vec![floor];
        align_dates(&mut messages, Some(&timezone));
        align_dates(&mut floors, Some(&timezone));
        let mut n = vec![];
        merge(&mut n, messages, floors, |_| true);
        assert_eq!(n.len(), 2);
        assert_eq!(n[0].date, "2026-02-01");
        assert_eq!(n[1].date, "2026-02-02");
        assert_eq!(n.iter().map(|m| total(&m.tokens)).sum::<i64>(), 20);
    }
    #[test]
    fn header_requires_version_and_same_home() {
        let raw = br#"{"version":1,"home":"C:\\Users\\15pro","created_at":"2026-09-14T00:00:00Z","policy":"local","messages":[]}"#;
        assert!(header_matches(raw, r"C:\Users\15pro"));
        assert!(!header_matches(raw, r"D:\other"));
        let wrong = br#"{"version":2,"home":"C:\\Users\\15pro","messages":[]}"#;
        assert!(!header_matches(wrong, r"C:\Users\15pro"));
    }
}
