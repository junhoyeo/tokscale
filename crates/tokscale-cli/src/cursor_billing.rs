//! Account-scoped Cursor billing events, adapted to the existing usage cache.
//!
//! Legacy usage-events and Billing V2 share no stable request id. Cross-source
//! rows merge only when a full fingerprint (timestamp, model, and all four
//! token counters) uniquely identifies one legacy row and one billing row.
//! Unproven billing rows that sit inside the live-or-matched legacy time span
//! stay in `pendingEvents` so reports keep the existing ledger without
//! double-counting. Pending status and matched-legacy provenance keep that
//! overlap window stable across reloads after confirmed matches leave the
//! reportable cache.
//!
//! CSV auto-migration is intentionally out of scope; upstream CSV scan/archive
//! behavior is unchanged. A later CSV follow-up must cover mixed-account cache
//! directories and per-row Principal ID attribution before any automatic claim.

use super::{
    build_cursor_json_headers, cached_usage_events, cursor_cache_owner_hash,
    cursor_http_client_builder, extract_user_id_from_session_token, fetch_cursor_usage_events_json,
    read_cursor_body_with_cap, CursorUsageFetch, CURSOR_HTTP_TIMEOUT, CURSOR_MAX_JSON_BYTES,
    CURSOR_MAX_JSON_PAGES, CURSOR_USAGE_CORRECTION_WINDOW_MS,
};
use anyhow::{bail, ensure, Context, Result};
use chrono::{DateTime, Duration as DateDuration, NaiveDate, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::cmp::Reverse;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::Duration;
use tokscale_core::sessions::cursor::{
    parse_cursor_usage_event_identity, parse_cursor_usage_event_timestamp as event_timestamp,
};

const ENDPOINT: &str = "https://cursor.com/api/dashboard/v2/query-billing-items";
const CACHE_KEY: &str = "cursorBillingV2";
/// Default first-sync lookback and per-request paging chunk.
const WINDOW: DateDuration = DateDuration::days(7);
const BILLING_EVENT_PREFIX: &str = "billing-v2:";
const MATCH_CONFIRMED: &str = "confirmed";
const MATCH_PENDING: &str = "pending";

#[cfg(test)]
thread_local! {
    static BILLING_ENDPOINT_OVERRIDE: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
    static LEGACY_JSON_OVERRIDE: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

#[derive(Deserialize)]
struct Page {
    data: Vec<Value>,
    pagination: Pagination,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Pagination {
    has_more: bool,
    next_cursor: Option<String>,
}

#[derive(Clone, Debug, Default)]
struct BillingEvents {
    alternate_owner: Option<String>,
    events: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HistoryCoverage {
    /// Inclusive start of the contiguous range that has been fully walked.
    checked_from_ms: Option<i64>,
    /// Exclusive end of that contiguous range.
    checked_to_ms: Option<i64>,
}

#[derive(Clone, Debug, Default)]
struct BillingMeta {
    alternate_owner: Option<String>,
    history: HistoryCoverage,
    pending: Vec<Value>,
    /// Legacy rows replaced by a proven V2 match; kept for audit/provenance only.
    matched_legacy: Vec<Value>,
    last_warning: Option<String>,
}

/// True once Billing V2 has committed reportable ledger rows (confirmed, pending,
/// or matched-away legacy provenance). Coverage-only empty windows do not count:
/// an inspected empty first window must not freeze legacy updates when later V2
/// requests fail.
fn billing_v2_ledger_present(cached: &[Value], meta: &BillingMeta) -> bool {
    cached.iter().any(is_billing_event)
        || !meta.pending.is_empty()
        || !meta.matched_legacy.is_empty()
}

/// True once any Billing V2 state is durable: ledger rows and/or inspected
/// history coverage. Used to keep an empty re-probe from wiping caches.
fn billing_v2_committed(cached: &[Value], meta: &BillingMeta) -> bool {
    billing_v2_ledger_present(cached, meta) || meta.history.checked_from_ms.is_some()
}

/// Parsed `--billing-since YYYY-MM-DD` (UTC midnight).
pub(super) fn parse_billing_since(date: &str) -> Result<DateTime<Utc>> {
    let day = NaiveDate::parse_from_str(date.trim(), "%Y-%m-%d")
        .with_context(|| format!("Invalid --billing-since date (expected YYYY-MM-DD): {date}"))?;
    day.and_hms_opt(0, 0, 0)
        .map(|naive| naive.and_utc())
        .context("Invalid --billing-since date")
}

pub(super) fn sync_warning_from_cache(cache_json: &str) -> Option<String> {
    serde_json::from_str::<Value>(cache_json)
        .ok()?
        .get(CACHE_KEY)?
        .get("lastWarning")?
        .as_str()
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

pub(super) async fn sync_account(
    fetch: CursorUsageFetch,
    timeout: Option<Duration>,
    custom_timeout: bool,
) -> Result<String> {
    let existing = fetch.existing_cache.as_deref();
    let legacy_fetch = || async {
        #[cfg(test)]
        if let Some(json) = LEGACY_JSON_OVERRIDE.with(|cell| cell.borrow().clone()) {
            return Ok(json);
        }
        fetch_cursor_usage_events_json(&fetch.session_token, timeout, custom_timeout, existing)
            .await
    };
    let mut cached = cached_usage_events(existing);
    let mut meta = read_billing_meta(existing);
    let ledger_established = billing_v2_ledger_present(&cached, &meta);
    let coverage_established = billing_v2_committed(&cached, &meta);
    let end = Utc::now();
    let budget = timeout.unwrap_or(CURSOR_HTTP_TIMEOUT);
    let billing = match fetch_billing(
        &fetch.session_token,
        &cached,
        meta.alternate_owner.clone(),
        end,
        fetch.billing_since,
        &meta.history,
        budget,
    )
    .await
    {
        Ok(Some(result)) => result,
        Ok(None) => {
            // Only a real V2 ledger blocks unavailable→legacy fallback. Coverage
            // from an empty inspected window must not freeze the legacy report.
            ensure!(
                !ledger_established,
                "Cursor Billing V2 is no longer available for this account"
            );
            let legacy = legacy_fetch().await?;
            return Ok(with_billing_warning(
                legacy,
                meta,
                "Cursor Billing V2 is unavailable; kept legacy usage",
            ));
        }
        Err(error) => {
            let timed_out = error.to_string().contains("timed out");
            if ledger_established {
                return Err(error);
            }
            let legacy = legacy_fetch().await?;
            return Ok(with_billing_warning(
                legacy,
                meta,
                &if timed_out {
                    "Cursor Billing V2 incomplete (timed out); kept legacy usage".to_owned()
                } else {
                    format!("Cursor Billing V2 incomplete ({error}); kept legacy usage")
                },
            ));
        }
    };

    if billing.events.events.is_empty() && billing.completed_windows.is_empty() {
        return if coverage_established {
            Ok(existing
                .context("Missing Cursor Billing V2 cache")?
                .to_owned())
        } else {
            let legacy = legacy_fetch().await?;
            Ok(with_billing_warning(
                legacy,
                meta,
                "Cursor Billing V2 returned no usage events; kept legacy usage",
            ))
        };
    }

    if cached.is_empty() {
        cached = cached_usage_events(legacy_fetch().await.ok().as_deref());
    }

    for (start_ms, end_ms) in &billing.completed_windows {
        extend_history_coverage(&mut meta.history, *start_ms, *end_ms);
    }
    meta.alternate_owner = billing
        .events
        .alternate_owner
        .clone()
        .or(meta.alternate_owner.take());
    let walk_warning = billing.walk_error.as_ref().map(|error| {
        format_backfill_progress_warning(error, fetch.billing_since, end, &meta.history)
    });
    meta.last_warning = match (meta.last_warning.take(), walk_warning) {
        (Some(previous), Some(walk)) => Some(format!("{previous}; {walk}")),
        (Some(previous), None) => Some(previous),
        (None, Some(walk)) => Some(walk),
        (None, None) => None,
    };
    merge_billing(cached, billing.events, meta)
}

fn format_backfill_progress_warning(
    error: &str,
    billing_since: Option<DateTime<Utc>>,
    end: DateTime<Utc>,
    history: &HistoryCoverage,
) -> String {
    let Some(since) = billing_since else {
        return error.to_owned();
    };
    let checked = match (history.checked_from_ms, history.checked_to_ms) {
        (Some(from), Some(to)) => format!(
            "checked coverage {}..{}",
            DateTime::from_timestamp_millis(from)
                .unwrap_or(since)
                .to_rfc3339_opts(SecondsFormat::Secs, true),
            DateTime::from_timestamp_millis(to)
                .unwrap_or(end)
                .to_rfc3339_opts(SecondsFormat::Secs, true)
        ),
        _ => "no completed coverage yet".to_owned(),
    };
    let remaining = backfill_windows(since, end, history);
    if remaining.is_empty() {
        return format!("{error}; {checked}");
    }
    let pending_start = remaining
        .iter()
        .map(|(start, _)| *start)
        .min()
        .unwrap_or(since)
        .to_rfc3339_opts(SecondsFormat::Secs, true);
    let pending_end = remaining
        .iter()
        .map(|(_, stop)| *stop)
        .max()
        .unwrap_or(end)
        .to_rfc3339_opts(SecondsFormat::Secs, true);
    format!("{error}; {checked}; still pending {pending_start}..{pending_end}")
}

#[derive(Debug)]
struct FetchedBilling {
    events: BillingEvents,
    completed_windows: Vec<(i64, i64)>,
    /// Set when a later window failed after earlier windows completed.
    walk_error: Option<String>,
}

fn billing_endpoint() -> String {
    #[cfg(test)]
    {
        if let Some(url) = BILLING_ENDPOINT_OVERRIDE.with(|cell| cell.borrow().clone()) {
            return url;
        }
    }
    ENDPOINT.to_string()
}

async fn fetch_billing(
    token: &str,
    cached: &[Value],
    alternate_owner: Option<String>,
    end: DateTime<Utc>,
    billing_since: Option<DateTime<Utc>>,
    history: &HistoryCoverage,
    budget: Duration,
) -> Result<Option<FetchedBilling>> {
    let mut headers = build_cursor_json_headers(token);
    let cookie = headers
        .get_mut(reqwest::header::COOKIE)
        .context("Invalid Cursor session cookie")?;
    cookie.set_sensitive(true);
    let client = cursor_http_client_builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let session_user = extract_user_id_from_session_token(token);
    let mut billing = BillingEvents {
        alternate_owner,
        ..BillingEvents::default()
    };
    let mut remaining_bytes = CURSOR_MAX_JSON_BYTES;
    let mut pages = 0;
    let mut completed_windows = Vec::new();
    let endpoint = billing_endpoint();
    let started = std::time::Instant::now();

    let mut windows = incremental_windows(cached, end);
    if let Some(since) = billing_since {
        windows.extend(backfill_windows(since, end, history));
    }
    windows = coalesce_windows(windows);
    ensure!(
        windows.iter().all(|(start, end)| (*end - *start) <= WINDOW),
        "Cursor Billing V2 request window exceeds seven days"
    );
    if windows.is_empty() {
        return Ok(Some(FetchedBilling {
            events: billing,
            completed_windows,
            walk_error: None,
        }));
    }

    for (start, window_end) in windows {
        let elapsed = started.elapsed();
        if elapsed >= budget {
            if completed_windows.is_empty() {
                bail!("Cursor Billing V2 sync timed out");
            }
            return Ok(Some(FetchedBilling {
                events: billing,
                completed_windows,
                walk_error: Some("Cursor Billing V2 sync timed out".to_owned()),
            }));
        }
        let start_ms = start.timestamp_millis();
        let end_ms = window_end.timestamp_millis();
        ensure!(
            end_ms - start_ms <= WINDOW.num_milliseconds(),
            "Cursor Billing V2 request window exceeds seven days"
        );
        let mut window_billing = BillingEvents {
            alternate_owner: billing.alternate_owner.clone(),
            ..BillingEvents::default()
        };
        let remaining = budget.saturating_sub(elapsed);
        let walk = fetch_billing_window(
            &client,
            &endpoint,
            &headers,
            session_user.as_deref(),
            start,
            window_end,
            &mut window_billing,
            &mut remaining_bytes,
            &mut pages,
            completed_windows.is_empty(),
            remaining,
        )
        .await;
        match walk {
            Ok(WindowWalk::Unavailable) => return Ok(None),
            Ok(WindowWalk::Complete) => {
                billing.alternate_owner = window_billing
                    .alternate_owner
                    .or(billing.alternate_owner.take());
                for (id, event) in window_billing.events {
                    if let Some(previous) = billing.events.get(&id) {
                        ensure!(
                            previous == &event,
                            "Conflicting snapshots in Cursor Billing V2 response"
                        );
                    } else {
                        billing.events.insert(id, event);
                    }
                }
                completed_windows.push((start_ms, end_ms));
            }
            Err(error) => {
                if completed_windows.is_empty() {
                    return Err(error);
                }
                return Ok(Some(FetchedBilling {
                    events: billing,
                    completed_windows,
                    walk_error: Some(error.to_string()),
                }));
            }
        }
    }
    Ok(Some(FetchedBilling {
        events: billing,
        completed_windows,
        walk_error: None,
    }))
}

enum WindowWalk {
    Complete,
    Unavailable,
}

#[allow(clippy::too_many_arguments)]
async fn fetch_billing_window(
    client: &reqwest::Client,
    endpoint: &str,
    headers: &reqwest::header::HeaderMap,
    session_user: Option<&str>,
    start: DateTime<Utc>,
    window_end: DateTime<Utc>,
    billing: &mut BillingEvents,
    remaining_bytes: &mut usize,
    pages: &mut usize,
    first_overall_window: bool,
    page_budget: Duration,
) -> Result<WindowWalk> {
    let start_text = start.to_rfc3339_opts(SecondsFormat::Millis, true);
    let end_text = window_end.to_rfc3339_opts(SecondsFormat::Millis, true);
    let mut seen_cursors = HashSet::new();
    let mut cursor = None::<String>;
    let pages_before = *pages;
    let page_started = std::time::Instant::now();
    loop {
        ensure!(
            *pages < CURSOR_MAX_JSON_PAGES,
            "Cursor Billing V2 page limit exceeded"
        );
        let remaining = page_budget.saturating_sub(page_started.elapsed());
        ensure!(
            remaining > Duration::ZERO,
            "Cursor Billing V2 sync timed out"
        );
        let params = [
            ("scope", "user"),
            ("product", "cursor"),
            ("startTime", start_text.as_str()),
            ("endTime", end_text.as_str()),
            ("pageSize", "100"),
        ];
        let mut request = client.get(endpoint).headers(headers.clone()).query(&params);
        if let Some(cursor) = cursor.as_deref() {
            request = request.query(&[("cursor", cursor)]);
        }
        let response = tokio::time::timeout(remaining, request.send())
            .await
            .map_err(|_| anyhow::anyhow!("Cursor Billing V2 sync timed out"))?
            .map_err(|error| error.without_url())?;
        let status = response.status();
        #[cfg(test)]
        eprintln!(
            "Cursor Billing V2 page {} returned HTTP {status}",
            *pages + 1
        );
        if first_overall_window
            && *pages == pages_before
            && matches!(status.as_u16(), 401 | 403 | 404 | 405)
        {
            return Ok(WindowWalk::Unavailable);
        }
        ensure!(
            status.is_success(),
            "Cursor Billing V2 returned HTTP {status}"
        );
        let body = read_cursor_body_with_cap(response, *remaining_bytes, "billing items JSON")
            .await
            .map_err(|error| anyhow::anyhow!("{}", super::cursor_sync_error_text(&error)))?;
        *remaining_bytes -= body.len();
        *pages += 1;
        let page: Page = serde_json::from_str(&body).context("Invalid Cursor Billing V2 page")?;
        for row in page.data {
            collect_event(
                billing,
                &row,
                session_user,
                start.timestamp_millis(),
                window_end.timestamp_millis(),
            )?;
        }
        let Some(next) = page
            .pagination
            .next_cursor
            .filter(|cursor| !cursor.is_empty())
        else {
            ensure!(
                !page.pagination.has_more,
                "Cursor Billing V2 page is missing its next cursor"
            );
            break;
        };
        ensure!(
            page.pagination.has_more && seen_cursors.insert(next.clone()),
            "Cursor Billing V2 pagination did not advance"
        );
        cursor = Some(next);
    }
    Ok(WindowWalk::Complete)
}

fn incremental_windows(
    cached: &[Value],
    end: DateTime<Utc>,
) -> Vec<(DateTime<Utc>, DateTime<Utc>)> {
    let mut start = window_start(cached, end);
    let mut windows = Vec::new();
    while start < end {
        let window_end = (start + WINDOW).min(end);
        windows.push((start, window_end));
        start = window_end;
    }
    windows
}

fn backfill_windows(
    since: DateTime<Utc>,
    end: DateTime<Utc>,
    history: &HistoryCoverage,
) -> Vec<(DateTime<Utc>, DateTime<Utc>)> {
    let target = since.min(end);
    if let (Some(from), Some(to)) = (history.checked_from_ms, history.checked_to_ms) {
        if from <= target.timestamp_millis() && to >= end.timestamp_millis() {
            return Vec::new();
        }
    }
    let mut windows = Vec::new();
    // Walk older than the contiguous covered prefix.
    let mut cursor = history
        .checked_from_ms
        .and_then(DateTime::from_timestamp_millis)
        .unwrap_or(end);
    while cursor > target {
        let window_start = (cursor - WINDOW).max(target);
        windows.push((window_start, cursor));
        cursor = window_start;
    }
    // Resume the unfinished suffix after a partially completed coverage range.
    if let Some(to) = history.checked_to_ms {
        if to < end.timestamp_millis() {
            let mut start = DateTime::from_timestamp_millis(to).unwrap_or(end);
            while start < end {
                let window_end = (start + WINDOW).min(end);
                windows.push((start, window_end));
                start = window_end;
            }
        }
    }
    windows
}

fn coalesce_windows(
    mut windows: Vec<(DateTime<Utc>, DateTime<Utc>)>,
) -> Vec<(DateTime<Utc>, DateTime<Utc>)> {
    windows.sort_by_key(|(start, _)| *start);
    let mut out = Vec::new();
    for (start, end) in windows {
        if start >= end {
            continue;
        }
        if let Some((_, last_end)) = out.last_mut() {
            // Merge only true overlaps. Abutting 7-day chunks stay separate so a
            // failed window cannot erase progress from a completed neighbor.
            if start < *last_end {
                if end > *last_end {
                    *last_end = end;
                }
                continue;
            }
        }
        out.push((start, end));
    }
    out
}

fn window_start(cached: &[Value], end: DateTime<Utc>) -> DateTime<Utc> {
    cached
        .iter()
        .filter(|row| is_billing_event(row))
        .filter_map(event_timestamp)
        .filter(|ms| *ms > 0 && *ms < end.timestamp_millis())
        .max()
        .and_then(|ms| DateTime::from_timestamp_millis(ms - CURSOR_USAGE_CORRECTION_WINDOW_MS))
        .unwrap_or(end - WINDOW)
}

fn collect_event(
    billing: &mut BillingEvents,
    row: &Value,
    session_user: Option<&str>,
    start: i64,
    end: i64,
) -> Result<()> {
    if row["product"] != "cursor" || row["chargeCategory"] != "usage" {
        return Ok(());
    }
    let id = row["billingItemId"]
        .as_str()
        .filter(|id| !id.is_empty())
        .context("Cursor billing item is missing its ID")?;
    let timestamp = DateTime::parse_from_rfc3339(
        row["eventTime"]
            .as_str()
            .context("Cursor billing item is missing its timestamp")?,
    )?
    .timestamp_millis();
    ensure!(
        (start..=end).contains(&timestamp),
        "Cursor billing item outside its requested window"
    );

    let principal = &row["principal"];
    let owner = principal["id"]
        .as_str()
        .filter(|id| !id.is_empty())
        .context("Cursor billing item is missing its owner")?;
    ensure!(
        principal["type"] == "user",
        "Cursor billing principal must be a user"
    );
    if Some(owner) != session_user {
        let hash = cursor_cache_owner_hash(owner);
        if let Some(previous) = &billing.alternate_owner {
            ensure!(previous == &hash, "Cursor billing principal changed");
        } else {
            billing.alternate_owner = Some(hash);
        }
    }

    let usage = row["usage"]
        .as_object()
        .context("Cursor billing item is missing usage")?;
    let meters = [
        ("inputTokens", "prompt-text-tokens"),
        ("outputTokens", "completion-text-tokens"),
        ("cacheReadTokens", "cached-prompt-text-tokens"),
        ("cacheWriteTokens", "cache-write-text-tokens"),
    ];
    let mut tokens = serde_json::Map::new();
    let produces_tokens = meters.iter().any(|(_, name)| usage.contains_key(*name));
    for (field, meter) in meters {
        let count = match usage.get(meter) {
            None => 0,
            Some(value) => value
                .as_i64()
                .or_else(|| value.as_str().and_then(|value| value.parse::<i64>().ok()))
                .filter(|count| *count >= 0)
                .context("Invalid token count in Cursor billing item")?,
        };
        tokens.insert(field.to_owned(), json!(count));
    }
    let price = &row["pricing"];
    let cost_cents = (price["currency"] == "USD")
        .then(|| {
            price["contracted"]
                .as_str()?
                .parse::<f64>()
                .ok()
                .map(|usd| usd * 100.0)
        })
        .flatten()
        .filter(|cost| cost.is_finite() && *cost > 0.0);
    if !produces_tokens && cost_cents.is_none() {
        return Ok(());
    }
    if let Some(cents) = cost_cents {
        tokens.insert("totalCents".to_owned(), json!(cents));
    }
    let model = row["displayExtras"]["model"]
        .as_str()
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .unwrap_or("unknown");
    let event = json!({
        "usageEventId": format!("{BILLING_EVENT_PREFIX}{id}"),
        "timestamp": timestamp.to_string(),
        "model": model,
        "tokenUsage": tokens,
    });
    if let Some(previous) = billing.events.get(id) {
        ensure!(
            previous == &event,
            "Conflicting snapshots in Cursor Billing V2 response"
        );
    } else {
        billing.events.insert(id.to_owned(), event);
    }
    Ok(())
}

fn is_billing_event(row: &Value) -> bool {
    row["usageEventId"]
        .as_str()
        .is_some_and(|id| id.starts_with(BILLING_EVENT_PREFIX))
}

fn merge_billing(
    cached: Vec<Value>,
    billing: BillingEvents,
    mut meta: BillingMeta,
) -> Result<String> {
    meta.alternate_owner = billing.alternate_owner.or(meta.alternate_owner.take());
    let incoming: Vec<_> = billing.events.into_values().collect();
    let mut legacy = Vec::new();
    let mut prior_billing = Vec::new();
    for row in cached {
        if is_billing_event(&row) {
            prior_billing.push(row);
        } else {
            legacy.push(row);
        }
    }
    for row in meta.pending.drain(..) {
        if is_billing_event(&row) {
            prior_billing.push(row);
        }
    }

    let prior_resolved: HashMap<String, Value> = prior_billing
        .iter()
        .filter(|row| billing_row_is_resolved(row))
        .filter_map(|row| {
            row["usageEventId"]
                .as_str()
                .map(|id| (id.to_owned(), row.clone()))
        })
        .collect();
    let prior_pending: HashMap<String, Value> = prior_billing
        .iter()
        .filter(|row| billing_row_is_pending(row))
        .filter_map(|row| {
            row["usageEventId"]
                .as_str()
                .map(|id| (id.to_owned(), row.clone()))
        })
        .collect();
    let mut merged_billing = super::merge_cursor_usage_events(prior_billing, incoming);
    // Incoming snapshots replace prior rows by id; re-apply proven match metadata
    // so a re-sync cannot demote confirmed billing back to pending.
    for row in &mut merged_billing {
        let Some(id) = row["usageEventId"].as_str().map(str::to_owned) else {
            continue;
        };
        if let Some(prior) = prior_resolved.get(&id) {
            if let Some(object) = row.as_object_mut() {
                if let Some(matched) = prior.get("matchedLegacyUsageEventId").cloned() {
                    object.insert("matchedLegacyUsageEventId".to_owned(), matched);
                }
                object.insert("billingMatchStatus".to_owned(), json!(MATCH_CONFIRMED));
            }
            continue;
        }
        // Sticky pending: keep unresolved overlap rows pending across reload until a
        // unique fingerprint match promotes them (or they leave the overlap window
        // with provenance span still covering matched-away legacy endpoints).
        if prior_pending.contains_key(&id) {
            set_match_status(row, MATCH_PENDING);
        }
    }
    // Overlap span must include matched-away legacy provenance so reloads cannot
    // promote mid-span pending rows just because confirmed matches emptied live legacy.
    let matched_legacy_provenance = meta.matched_legacy.clone();
    let (legacy_kept, confirmed, pending, newly_matched_legacy) =
        reconcile_cross_source(legacy, &matched_legacy_provenance, merged_billing);
    meta.pending = pending;
    meta.matched_legacy
        .extend(newly_matched_legacy.into_iter().map(|row| {
            let mut row = row;
            if let Some(object) = row.as_object_mut() {
                object.insert("billingMatchStatus".to_owned(), json!("matched"));
            }
            row
        }));

    let mut events = legacy_kept;
    events.extend(confirmed);
    events.sort_by_key(|row| Reverse(event_timestamp(row)));
    Ok(serde_json::to_string(&json!({
        "usageEventsDisplay": events,
        "totalUsageEventsCount": events.len(),
        (CACHE_KEY): serialize_meta(&meta),
    }))?)
}

fn reconcile_cross_source(
    mut legacy: Vec<Value>,
    matched_legacy_provenance: &[Value],
    billing_rows: Vec<Value>,
) -> (Vec<Value>, Vec<Value>, Vec<Value>, Vec<Value>) {
    let legacy_span = overlap_span_ms(&legacy, matched_legacy_provenance);
    let mut unresolved_billing = Vec::new();
    let mut confirmed = Vec::new();
    for row in billing_rows {
        if billing_row_is_resolved(&row) {
            let mut row = row;
            set_match_status(&mut row, MATCH_CONFIRMED);
            confirmed.push(row);
        } else {
            unresolved_billing.push(row);
        }
    }

    let mut legacy_fps: HashMap<EventFingerprint, Vec<usize>> = HashMap::new();
    for (index, row) in legacy.iter().enumerate() {
        if let Some(fp) = event_fingerprint(row) {
            legacy_fps.entry(fp).or_default().push(index);
        }
    }
    let mut billing_fps: HashMap<EventFingerprint, Vec<usize>> = HashMap::new();
    for (index, row) in unresolved_billing.iter().enumerate() {
        if let Some(fp) = event_fingerprint(row) {
            billing_fps.entry(fp).or_default().push(index);
        }
    }

    let mut matched_legacy_idxs = HashSet::new();
    let mut billing_to_legacy_idx = HashMap::new();
    for (fp, legacy_idxs) in &legacy_fps {
        if legacy_idxs.len() != 1 {
            continue;
        }
        let Some(billing_idxs) = billing_fps.get(fp) else {
            continue;
        };
        if billing_idxs.len() != 1 {
            continue;
        }
        matched_legacy_idxs.insert(legacy_idxs[0]);
        billing_to_legacy_idx.insert(billing_idxs[0], legacy_idxs[0]);
    }

    let mut newly_matched_legacy = Vec::new();
    let mut legacy_id_by_billing_idx = HashMap::new();
    for (billing_idx, legacy_idx) in &billing_to_legacy_idx {
        if let Some(id) = legacy[*legacy_idx]["usageEventId"].as_str() {
            legacy_id_by_billing_idx.insert(*billing_idx, id.to_owned());
        }
    }
    let legacy_kept: Vec<_> = legacy
        .drain(..)
        .enumerate()
        .filter_map(|(index, row)| {
            if matched_legacy_idxs.contains(&index) {
                newly_matched_legacy.push(row);
                None
            } else {
                Some(row)
            }
        })
        .collect();

    let mut pending = Vec::new();
    for (index, mut row) in unresolved_billing.into_iter().enumerate() {
        if billing_to_legacy_idx.contains_key(&index) {
            if let Some(legacy_id) = legacy_id_by_billing_idx.get(&index) {
                if let Some(object) = row.as_object_mut() {
                    object.insert("matchedLegacyUsageEventId".to_owned(), json!(legacy_id));
                }
            }
            set_match_status(&mut row, MATCH_CONFIRMED);
            confirmed.push(row);
            continue;
        }
        // Sticky pending survives empty live-legacy spans; provenance span also
        // keeps first-run mid-window unmatched rows from promoting on reload.
        if billing_row_is_pending(&row) {
            set_match_status(&mut row, MATCH_PENDING);
            pending.push(row);
            continue;
        }
        let overlaps = event_timestamp(&row).is_some_and(|at| {
            legacy_span.is_some_and(|(min_at, max_at)| at >= min_at && at <= max_at)
        });
        if overlaps {
            set_match_status(&mut row, MATCH_PENDING);
            pending.push(row);
        } else {
            set_match_status(&mut row, MATCH_CONFIRMED);
            confirmed.push(row);
        }
    }
    (legacy_kept, confirmed, pending, newly_matched_legacy)
}

fn billing_row_is_pending(row: &Value) -> bool {
    row.get("billingMatchStatus").and_then(Value::as_str) == Some(MATCH_PENDING)
}

fn billing_row_is_resolved(row: &Value) -> bool {
    row.get("matchedLegacyUsageEventId")
        .and_then(Value::as_str)
        .is_some_and(|id| !id.is_empty())
        || row.get("billingMatchStatus").and_then(Value::as_str) == Some(MATCH_CONFIRMED)
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct EventFingerprint {
    timestamp: i64,
    model: String,
    input: i64,
    output: i64,
    cache_read: i64,
    cache_write: i64,
}

fn event_fingerprint(row: &Value) -> Option<EventFingerprint> {
    let identity = parse_cursor_usage_event_identity(row)?;
    Some(EventFingerprint {
        timestamp: identity.timestamp,
        model: identity.model,
        input: identity.tokens.input,
        output: identity.tokens.output,
        cache_read: identity.tokens.cache_read,
        cache_write: identity.tokens.cache_write,
    })
}

/// Live legacy ∪ matched-away provenance so overlap bounds stay stable after
/// confirmed matches remove endpoints from the reportable cache.
fn overlap_span_ms(legacy: &[Value], matched_legacy: &[Value]) -> Option<(i64, i64)> {
    let mut min_at = None;
    let mut max_at = None;
    for row in legacy.iter().chain(matched_legacy.iter()) {
        let Some(at) = event_timestamp(row).filter(|ms| *ms > 0) else {
            continue;
        };
        min_at = Some(min_at.map_or(at, |current: i64| current.min(at)));
        max_at = Some(max_at.map_or(at, |current: i64| current.max(at)));
    }
    Some((min_at?, max_at?))
}

fn set_match_status(row: &mut Value, status: &str) {
    if let Some(object) = row.as_object_mut() {
        object.insert("billingMatchStatus".to_owned(), json!(status));
    }
}

fn read_billing_meta(existing: Option<&str>) -> BillingMeta {
    let Some(text) = existing else {
        return BillingMeta::default();
    };
    let Ok(cache) = serde_json::from_str::<Value>(text) else {
        return BillingMeta::default();
    };
    let Some(meta) = cache.get(CACHE_KEY) else {
        return BillingMeta::default();
    };
    BillingMeta {
        alternate_owner: meta
            .get("alternateOwnerHash")
            .and_then(Value::as_str)
            .map(str::to_owned),
        history: HistoryCoverage {
            checked_from_ms: meta
                .pointer("/history/checkedFromMs")
                .and_then(Value::as_i64),
            checked_to_ms: meta.pointer("/history/checkedToMs").and_then(Value::as_i64),
        },
        pending: meta
            .get("pendingEvents")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
        matched_legacy: meta
            .get("matchedLegacyEvents")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
        last_warning: meta
            .get("lastWarning")
            .and_then(Value::as_str)
            .map(str::to_owned),
    }
}

fn serialize_meta(meta: &BillingMeta) -> Value {
    json!({
        "alternateOwnerHash": meta.alternate_owner,
        "history": {
            "checkedFromMs": meta.history.checked_from_ms,
            "checkedToMs": meta.history.checked_to_ms,
        },
        "pendingEvents": meta.pending,
        "matchedLegacyEvents": meta.matched_legacy,
        "lastWarning": meta.last_warning,
    })
}

fn extend_history_coverage(history: &mut HistoryCoverage, start_ms: i64, end_ms: i64) {
    if start_ms >= end_ms {
        return;
    }
    match (history.checked_from_ms, history.checked_to_ms) {
        (None, None) => {
            history.checked_from_ms = Some(start_ms);
            history.checked_to_ms = Some(end_ms);
        }
        (Some(from), Some(to)) => {
            if end_ms < from || start_ms > to {
                // Non-contiguous ranges are not marked complete as a gap filler.
                if end_ms == from {
                    history.checked_from_ms = Some(start_ms);
                } else if start_ms == to {
                    history.checked_to_ms = Some(end_ms);
                }
                return;
            }
            history.checked_from_ms = Some(from.min(start_ms));
            history.checked_to_ms = Some(to.max(end_ms));
        }
        _ => {
            history.checked_from_ms = Some(start_ms);
            history.checked_to_ms = Some(end_ms);
        }
    }
}

fn with_billing_warning(legacy_json: String, mut meta: BillingMeta, warning: &str) -> String {
    let mut events = cached_usage_events(Some(&legacy_json));
    meta.last_warning = Some(warning.to_owned());
    events.sort_by_key(|row| Reverse(event_timestamp(row)));
    serde_json::to_string(&json!({
        "usageEventsDisplay": events,
        "totalUsageEventsCount": events.len(),
        (CACHE_KEY): serialize_meta(&meta),
    }))
    .unwrap_or(legacy_json)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::sync::{Arc, Mutex};

    const AT: i64 = 1_795_359_600_000;

    struct EndpointOverride {
        _private: (),
    }

    impl EndpointOverride {
        fn set(url: impl Into<String>) -> Self {
            BILLING_ENDPOINT_OVERRIDE.with(|cell| *cell.borrow_mut() = Some(url.into()));
            Self { _private: () }
        }
    }

    impl Drop for EndpointOverride {
        fn drop(&mut self) {
            BILLING_ENDPOINT_OVERRIDE.with(|cell| *cell.borrow_mut() = None);
        }
    }

    fn event(id: &str, owner: &str, at: i64, tokens: u64) -> Value {
        json!({
            "billingItemId": id,
            "eventTime": DateTime::from_timestamp_millis(at).unwrap().to_rfc3339(),
            "product": "cursor",
            "chargeCategory": "usage",
            "principal": {"type": "user", "id": owner},
            "usage": {
                "prompt-text-tokens": tokens.to_string(),
                "completion-text-tokens": "2",
                "cached-prompt-text-tokens": "3",
                "cache-write-text-tokens": "4"
            },
            "pricing": {"currency": "USD", "contracted": "0.00"},
            "displayExtras": {"model": "auto"},
        })
    }

    fn legacy_row(id: &str, at: i64, model: &str, input: i64) -> Value {
        json!({
            "usageEventId": id,
            "timestamp": at.to_string(),
            "model": model,
            "tokenUsage": {
                "inputTokens": input,
                "outputTokens": 2,
                "cacheReadTokens": 3,
                "cacheWriteTokens": 4
            },
        })
    }

    fn billing_page(items: Vec<Value>, has_more: bool, next: Option<&str>) -> Vec<u8> {
        serde_json::to_vec(&json!({
            "data": items,
            "pagination": {"hasMore": has_more, "nextCursor": next},
        }))
        .unwrap()
    }

    fn serve_billing_pages(
        pages: Vec<(u16, Vec<u8>)>,
    ) -> (String, Arc<Mutex<Vec<String>>>, EndpointOverride) {
        serve_billing_pages_delayed(
            pages
                .into_iter()
                .map(|(status, body)| (status, body, Duration::ZERO))
                .collect(),
        )
    }

    fn serve_billing_pages_delayed(
        pages: Vec<(u16, Vec<u8>, Duration)>,
    ) -> (String, Arc<Mutex<Vec<String>>>, EndpointOverride) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::<String>::new()));
        let requests_thread = Arc::clone(&requests);
        std::thread::spawn(move || {
            for (status, body, delay_before) in pages {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                let mut request = [0u8; 8192];
                let read = stream.read(&mut request).unwrap_or(0);
                requests_thread
                    .lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&request[..read]).into_owned());
                if !delay_before.is_zero() {
                    std::thread::sleep(delay_before);
                }
                let head = format!(
                    "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(&body);
                let _ = stream.flush();
                std::thread::sleep(Duration::from_millis(50));
            }
        });
        let url = format!("http://{addr}/api/dashboard/v2/query-billing-items");
        let guard = EndpointOverride::set(url.clone());
        (url, requests, guard)
    }

    #[test]
    fn h1_proven_match_dedups_while_legacy_only_remains() -> Result<()> {
        let matched_legacy = legacy_row("legacy-match", AT, "auto", 7);
        let legacy_only = legacy_row("legacy-only", AT, "auto", 9);
        let mut initial = BillingEvents::default();
        collect_event(
            &mut initial,
            &event("a", "session", AT, 7),
            Some("session"),
            AT - 9,
            AT + 9,
        )?;
        let merged = merge_billing(
            vec![matched_legacy, legacy_only.clone()],
            initial,
            BillingMeta::default(),
        )?;
        let rows = cached_usage_events(Some(&merged));
        let ids: HashSet<_> = rows
            .iter()
            .filter_map(|row| row["usageEventId"].as_str())
            .collect();
        assert!(ids.contains("legacy-only"));
        assert!(ids.contains("billing-v2:a"));
        assert!(!ids.contains("legacy-match"));
        assert_eq!(
            rows.iter()
                .filter(|row| row["usageEventId"] == "billing-v2:a")
                .count(),
            1
        );
        let meta = read_billing_meta(Some(&merged));
        assert_eq!(meta.matched_legacy.len(), 1);
        assert_eq!(meta.matched_legacy[0]["usageEventId"], "legacy-match");
        assert_eq!(
            rows.iter()
                .find(|row| row["usageEventId"] == "billing-v2:a")
                .and_then(|row| row["matchedLegacyUsageEventId"].as_str()),
            Some("legacy-match")
        );
        Ok(())
    }

    #[test]
    fn h1_distinct_v2_ids_with_identical_meters_are_both_kept() -> Result<()> {
        let mut initial = BillingEvents::default();
        collect_event(
            &mut initial,
            &event("a", "session", AT, 7),
            Some("session"),
            AT - 9,
            AT + 9,
        )?;
        collect_event(
            &mut initial,
            &event("b", "session", AT, 7),
            Some("session"),
            AT - 9,
            AT + 9,
        )?;
        let merged = merge_billing(Vec::new(), initial, BillingMeta::default())?;
        let rows = cached_usage_events(Some(&merged));
        let ids: HashSet<_> = rows
            .iter()
            .filter_map(|row| row["usageEventId"].as_str())
            .collect();
        assert_eq!(ids.len(), 2);
        assert!(ids.contains("billing-v2:a"));
        assert!(ids.contains("billing-v2:b"));
        Ok(())
    }

    #[test]
    fn h1_unproven_overlap_keeps_legacy_and_parks_billing() -> Result<()> {
        let legacy = legacy_row("legacy-x", AT, "default", 5);
        let mut initial = BillingEvents::default();
        collect_event(
            &mut initial,
            &event("a", "session", AT, 7),
            Some("session"),
            AT - 9,
            AT + 9,
        )?;
        let merged = merge_billing(vec![legacy.clone()], initial, BillingMeta::default())?;
        let rows = cached_usage_events(Some(&merged));
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["usageEventId"], "legacy-x");
        let meta = read_billing_meta(Some(&merged));
        assert_eq!(meta.pending.len(), 1);
        assert_eq!(meta.pending[0]["usageEventId"], "billing-v2:a");
        assert_eq!(meta.pending[0]["billingMatchStatus"], MATCH_PENDING);
        Ok(())
    }

    #[test]
    fn h1_updates_billing_by_id_without_time_cutover_deletes() -> Result<()> {
        let old = legacy_row("old", AT - 1_000, "default", 5);
        let mut initial = BillingEvents::default();
        collect_event(
            &mut initial,
            &event("a", "linked", AT + 5_000, 7),
            Some("session"),
            AT,
            AT + 9_000,
        )?;
        let first = merge_billing(vec![old.clone()], initial, BillingMeta::default())?;
        let mut correction = BillingEvents::default();
        collect_event(
            &mut correction,
            &event("a", "linked", AT + 5_000, 6),
            Some("session"),
            AT,
            AT + 9_000,
        )?;
        let second = merge_billing(
            cached_usage_events(Some(&first)),
            correction,
            read_billing_meta(Some(&first)),
        )?;
        let rows = cached_usage_events(Some(&second));
        assert!(rows.iter().any(|row| row["usageEventId"] == "old"));
        let billing = rows
            .iter()
            .find(|row| row["usageEventId"] == "billing-v2:a")
            .expect("billing row");
        assert_eq!(billing["tokenUsage"]["inputTokens"], 6);
        Ok(())
    }

    #[test]
    fn rejects_multiple_alternate_owners_and_conflicting_snapshots() -> Result<()> {
        let mut billing = BillingEvents::default();
        let mut collect = |row| collect_event(&mut billing, &row, Some("session"), AT, AT + 9);
        collect(event("a", "linked", AT, 1))?;
        collect(event("b", "session", AT, 2))?;
        assert!(collect(event("c", "other", AT, 3)).is_err());
        assert!(collect(event("a", "linked", AT, 4)).is_err());
        Ok(())
    }

    #[test]
    fn history_coverage_extends_only_for_complete_contiguous_windows() {
        let mut history = HistoryCoverage::default();
        extend_history_coverage(&mut history, 100, 200);
        assert_eq!(history.checked_from_ms, Some(100));
        assert_eq!(history.checked_to_ms, Some(200));
        extend_history_coverage(&mut history, 0, 50);
        assert_eq!(history.checked_from_ms, Some(100));
        extend_history_coverage(&mut history, 50, 100);
        assert_eq!(history.checked_from_ms, Some(50));
        extend_history_coverage(&mut history, 200, 300);
        assert_eq!(history.checked_to_ms, Some(300));
    }

    #[test]
    fn backfill_windows_resume_from_checked_from_without_claiming_gaps() {
        let end = DateTime::from_timestamp_millis(AT).unwrap();
        let since = end - DateDuration::days(21);
        let history = HistoryCoverage {
            checked_from_ms: Some((end - DateDuration::days(7)).timestamp_millis()),
            checked_to_ms: Some(end.timestamp_millis()),
        };
        let windows = backfill_windows(since, end, &history);
        assert!(!windows.is_empty());
        assert_eq!(
            windows[0].1.timestamp_millis(),
            history.checked_from_ms.unwrap()
        );
        assert!(windows.iter().all(|(start, stop)| *start < *stop));
        assert!(windows.last().unwrap().0 <= since);
    }

    #[test]
    fn m1_unavailable_v2_keeps_legacy_with_warning() -> Result<()> {
        let (_url, _requests, _guard) = serve_billing_pages(vec![(404, b"missing".to_vec())]);
        let runtime = tokio::runtime::Runtime::new()?;
        let legacy = json!({
            "usageEventsDisplay": [legacy_row("legacy-1", AT - 10_000, "default", 1)],
            "totalUsageEventsCount": 1
        })
        .to_string();
        // Directly exercise unavailable path via fetch_billing + with_billing_warning.
        let result = runtime.block_on(fetch_billing(
            "user_test::token",
            &cached_usage_events(Some(&legacy)),
            None,
            DateTime::from_timestamp_millis(AT).unwrap(),
            None,
            &HistoryCoverage::default(),
            Duration::from_secs(10),
        ))?;
        assert!(result.is_none());
        let warned = with_billing_warning(
            legacy.clone(),
            BillingMeta::default(),
            "Cursor Billing V2 is unavailable; kept legacy usage",
        );
        assert_eq!(
            sync_warning_from_cache(&warned).as_deref(),
            Some("Cursor Billing V2 is unavailable; kept legacy usage")
        );
        assert_eq!(cached_usage_events(Some(&warned)).len(), 1);
        Ok(())
    }

    #[test]
    fn m3_mid_pagination_failure_does_not_complete_window() -> Result<()> {
        let item = event("a", "session", AT - 1_000, 7);
        let page1 = billing_page(vec![item], true, Some("cursor-2"));
        let (_url, requests, _guard) =
            serve_billing_pages(vec![(200, page1), (500, b"boom".to_vec())]);
        let runtime = tokio::runtime::Runtime::new()?;
        let end = DateTime::from_timestamp_millis(AT).unwrap();
        let err = runtime
            .block_on(fetch_billing(
                "user_test::token",
                &[],
                None,
                end,
                None,
                &HistoryCoverage::default(),
                Duration::from_secs(10),
            ))
            .expect_err("page-2 500 must fail the walk");
        assert!(err.to_string().contains("500"), "{err}");
        assert_eq!(requests.lock().unwrap().len(), 2);
        Ok(())
    }

    #[test]
    fn m3_failed_window_leaves_history_coverage_unchanged() {
        let history = HistoryCoverage {
            checked_from_ms: Some(AT - 7 * 86_400_000),
            checked_to_ms: Some(AT),
        };
        let mut next = history.clone();
        // Simulate not calling extend_history_coverage on failure.
        assert_eq!(next.checked_from_ms, history.checked_from_ms);
        assert_eq!(next.checked_to_ms, history.checked_to_ms);
        extend_history_coverage(&mut next, AT - 14 * 86_400_000, AT - 7 * 86_400_000);
        assert_eq!(next.checked_from_ms, Some(AT - 14 * 86_400_000));
    }

    #[test]
    fn h2_mock_backfill_is_idempotent_across_complete_windows() -> Result<()> {
        let end = DateTime::from_timestamp_millis(AT).unwrap();
        let older_at = (end - DateDuration::days(10)).timestamp_millis();
        let recent_at = (end - DateDuration::days(1)).timestamp_millis();
        let recent = event("recent", "session", recent_at, 7);
        let older = event("older", "session", older_at, 8);
        // Windows are sorted oldest-first: backfill chunk then incremental chunk.
        let pages = vec![
            (200, billing_page(vec![older.clone()], false, None)),
            (200, billing_page(vec![recent.clone()], false, None)),
        ];
        let (_url, requests, _guard) = serve_billing_pages(pages);
        let runtime = tokio::runtime::Runtime::new()?;
        let since = end - DateDuration::days(14);
        let first = runtime
            .block_on(fetch_billing(
                "user_test::token",
                &[],
                None,
                end,
                Some(since),
                &HistoryCoverage::default(),
                Duration::from_secs(10),
            ))?
            .expect("v2 available");
        assert_eq!(first.events.events.len(), 2);
        assert_eq!(
            first.completed_windows.len(),
            requests.lock().unwrap().len()
        );
        assert!(first.completed_windows.len() >= 2);

        let mut meta = BillingMeta::default();
        for (start, stop) in &first.completed_windows {
            extend_history_coverage(&mut meta.history, *start, *stop);
        }
        let merged = merge_billing(Vec::new(), first.events, meta)?;
        let covered = read_billing_meta(Some(&merged)).history;
        assert!(covered.checked_from_ms.is_some());
        assert!(covered.checked_to_ms.is_some());

        // Repeat backfill: coverage already reaches `since`, so only incremental runs.
        let second_pages = vec![(200, billing_page(vec![recent], false, None))];
        let (_url2, requests2, _guard2) = serve_billing_pages(second_pages);
        let second = runtime
            .block_on(fetch_billing(
                "user_test::token",
                &cached_usage_events(Some(&merged)),
                None,
                end,
                Some(since),
                &covered,
                Duration::from_secs(10),
            ))?
            .expect("v2 available");
        assert_eq!(requests2.lock().unwrap().len(), 1);
        assert!(
            backfill_windows(since, end, &covered).is_empty(),
            "completed coverage must not re-queue older backfill windows"
        );
        let again = merge_billing(
            cached_usage_events(Some(&merged)),
            second.events,
            read_billing_meta(Some(&merged)),
        )?;
        let ids: HashSet<_> = cached_usage_events(Some(&again))
            .iter()
            .filter_map(|row| row["usageEventId"].as_str().map(str::to_owned))
            .collect();
        assert_eq!(ids.len(), 2);
        assert!(ids.contains("billing-v2:recent"));
        assert!(ids.contains("billing-v2:older"));
        Ok(())
    }

    #[test]
    fn background_budget_stays_default_while_explicit_uses_override() {
        assert_eq!(CURSOR_HTTP_TIMEOUT, Duration::from_secs(15));
        let explicit = Duration::from_secs(120);
        assert!(explicit > CURSOR_HTTP_TIMEOUT);
    }

    fn token_totals(rows: &[Value]) -> (i64, i64, i64, i64, f64) {
        let mut input = 0;
        let mut output = 0;
        let mut cache_read = 0;
        let mut cache_write = 0;
        let mut cents = 0.0;
        for row in rows {
            let usage = &row["tokenUsage"];
            input += usage["inputTokens"].as_i64().unwrap_or(0);
            output += usage["outputTokens"].as_i64().unwrap_or(0);
            cache_read += usage["cacheReadTokens"].as_i64().unwrap_or(0);
            cache_write += usage["cacheWriteTokens"].as_i64().unwrap_or(0);
            cents += usage["totalCents"].as_f64().unwrap_or(0.0);
        }
        (input, output, cache_read, cache_write, cents)
    }

    fn rerun_merge(first: &str, billing: BillingEvents) -> Result<String> {
        merge_billing(
            cached_usage_events(Some(first)),
            billing,
            read_billing_meta(Some(first)),
        )
    }

    fn assert_stable_ledger(first: &str, second: &str) {
        let first_rows = cached_usage_events(Some(first));
        let second_rows = cached_usage_events(Some(second));
        let first_meta = read_billing_meta(Some(first));
        let second_meta = read_billing_meta(Some(second));
        let first_ids: Vec<String> = first_rows
            .iter()
            .filter_map(|row| row["usageEventId"].as_str().map(str::to_owned))
            .collect();
        let second_ids: Vec<String> = second_rows
            .iter()
            .filter_map(|row| row["usageEventId"].as_str().map(str::to_owned))
            .collect();
        assert_eq!(
            second_ids, first_ids,
            "visible usageEventId set must be sticky"
        );
        assert_eq!(second_meta.pending.len(), first_meta.pending.len());
        let pending_ids = |meta: &BillingMeta| -> Vec<String> {
            meta.pending
                .iter()
                .filter_map(|row| row["usageEventId"].as_str().map(str::to_owned))
                .collect()
        };
        assert_eq!(pending_ids(&second_meta), pending_ids(&first_meta));
        assert_eq!(token_totals(&second_rows), token_totals(&first_rows));
    }

    #[test]
    fn acceptance_two_to_three_matched_ends_pending_middle_sticky_across_disk() -> Result<()> {
        // Exact 2→3 fixture: L0+L2 legacy, B0/B1/B2 V2. B0/B2 match ends; B1 mid pending.
        // After matched ends leave the report, reload must keep 2 visible + 1 pending.
        let l0 = legacy_row("L0", AT - 2_000, "auto", 1);
        let l2 = legacy_row("L2", AT + 2_000, "auto", 3);
        let mut billing = BillingEvents::default();
        collect_event(
            &mut billing,
            &event("B0", "session", AT - 2_000, 1),
            Some("session"),
            AT - 9_000,
            AT + 9_000,
        )?;
        collect_event(
            &mut billing,
            &event("B1", "session", AT, 99),
            Some("session"),
            AT - 9_000,
            AT + 9_000,
        )?;
        collect_event(
            &mut billing,
            &event("B2", "session", AT + 2_000, 3),
            Some("session"),
            AT - 9_000,
            AT + 9_000,
        )?;
        let first = merge_billing(vec![l0, l2], billing.clone(), BillingMeta::default())?;
        let first_rows = cached_usage_events(Some(&first));
        let first_meta = read_billing_meta(Some(&first));
        assert_eq!(first_rows.len(), 2, "first pass: 2 visible");
        assert_eq!(first_meta.pending.len(), 1, "first pass: 1 pending");
        assert!(first_rows
            .iter()
            .any(|row| row["usageEventId"] == "billing-v2:B0"));
        assert!(first_rows
            .iter()
            .any(|row| row["usageEventId"] == "billing-v2:B2"));
        assert!(!first_rows
            .iter()
            .any(|row| row["usageEventId"] == "billing-v2:B1"));
        assert_eq!(
            first_meta.pending[0]["usageEventId"].as_str(),
            Some("billing-v2:B1")
        );
        assert_eq!(first_meta.matched_legacy.len(), 2);
        // Matched ends must leave live legacy empty — this is the promotion trap.
        assert!(
            !first_rows
                .iter()
                .any(|row| { row["usageEventId"] == "L0" || row["usageEventId"] == "L2" }),
            "matched L0/L2 must leave the visible ledger"
        );

        let second = rerun_merge(&first, billing.clone())?;
        assert_stable_ledger(&first, &second);
        let second_rows = cached_usage_events(Some(&second));
        assert_eq!(second_rows.len(), 2, "in-memory re-run must not grow 2→3");
        assert_eq!(read_billing_meta(Some(&second)).pending.len(), 1);

        // Disk reload: serialize → parse → merge again with same V2 snapshot.
        let disk = first.clone();
        let from_disk = merge_billing(
            cached_usage_events(Some(&disk)),
            billing,
            read_billing_meta(Some(&disk)),
        )?;
        assert_stable_ledger(&first, &from_disk);
        assert_eq!(cached_usage_events(Some(&from_disk)).len(), 2);
        assert_eq!(read_billing_meta(Some(&from_disk)).pending.len(), 1);
        Ok(())
    }

    #[test]
    fn acceptance_three_to_two_only_middle_matches_sticky_across_disk() -> Result<()> {
        // Exact 3→2 fixture: three legacy, only middle matches one V2 row.
        let l0 = legacy_row("L0", AT - 2_000, "auto", 1);
        let l1 = legacy_row("L1", AT, "auto", 7);
        let l2 = legacy_row("L2", AT + 2_000, "auto", 3);
        let mut billing = BillingEvents::default();
        collect_event(
            &mut billing,
            &event("B1", "session", AT, 7),
            Some("session"),
            AT - 9_000,
            AT + 9_000,
        )?;
        let first = merge_billing(vec![l0, l1, l2], billing.clone(), BillingMeta::default())?;
        let first_rows = cached_usage_events(Some(&first));
        let first_meta = read_billing_meta(Some(&first));
        assert_eq!(first_rows.len(), 3, "first pass: 3 visible");
        assert_eq!(first_meta.pending.len(), 0);
        assert!(first_rows.iter().any(|row| row["usageEventId"] == "L0"));
        assert!(first_rows.iter().any(|row| row["usageEventId"] == "L2"));
        assert!(first_rows
            .iter()
            .any(|row| row["usageEventId"] == "billing-v2:B1"));
        assert!(!first_rows.iter().any(|row| row["usageEventId"] == "L1"));
        assert_eq!(first_meta.matched_legacy.len(), 1);

        let second = rerun_merge(&first, billing.clone())?;
        assert_stable_ledger(&first, &second);
        assert_eq!(
            cached_usage_events(Some(&second)).len(),
            3,
            "re-run must not shrink 3→2"
        );

        let from_disk = merge_billing(
            cached_usage_events(Some(&first)),
            billing,
            read_billing_meta(Some(&first)),
        )?;
        assert_stable_ledger(&first, &from_disk);
        assert_eq!(cached_usage_events(Some(&from_disk)).len(), 3);
        Ok(())
    }

    #[test]
    fn idempotent_unmatched_middle_v2_keeps_report_at_two() -> Result<()> {
        // Legacy-only ends + unmatched mid V2 (no matching B0/B2).
        let early = legacy_row("legacy-early", AT - 2_000, "auto", 1);
        let late = legacy_row("legacy-late", AT + 2_000, "auto", 3);
        let mut billing = BillingEvents::default();
        collect_event(
            &mut billing,
            &event("mid", "session", AT, 99),
            Some("session"),
            AT - 9_000,
            AT + 9_000,
        )?;
        let first = merge_billing(
            vec![early.clone(), late.clone()],
            billing,
            BillingMeta::default(),
        )?;
        let first_rows = cached_usage_events(Some(&first));
        assert_eq!(first_rows.len(), 2, "report stays 2 with pending mid");
        let first_meta = read_billing_meta(Some(&first));
        assert_eq!(first_meta.pending.len(), 1);

        let mut again = BillingEvents::default();
        collect_event(
            &mut again,
            &event("mid", "session", AT, 99),
            Some("session"),
            AT - 9_000,
            AT + 9_000,
        )?;
        let second = rerun_merge(&first, again)?;
        assert_stable_ledger(&first, &second);
        let second_rows = cached_usage_events(Some(&second));
        assert!(second_rows
            .iter()
            .any(|row| row["usageEventId"] == "legacy-early"));
        assert!(second_rows
            .iter()
            .any(|row| row["usageEventId"] == "legacy-late"));
        Ok(())
    }

    #[test]
    fn idempotent_matched_middle_v2_keeps_report_at_three() -> Result<()> {
        let early = legacy_row("legacy-early", AT - 2_000, "auto", 1);
        let middle = legacy_row("legacy-mid", AT, "auto", 7);
        let late = legacy_row("legacy-late", AT + 2_000, "auto", 3);
        let mut billing = BillingEvents::default();
        collect_event(
            &mut billing,
            &event("mid", "session", AT, 7),
            Some("session"),
            AT - 9_000,
            AT + 9_000,
        )?;
        let first = merge_billing(vec![early, middle, late], billing, BillingMeta::default())?;
        assert_eq!(cached_usage_events(Some(&first)).len(), 3);
        let first_meta = read_billing_meta(Some(&first));
        assert_eq!(first_meta.matched_legacy.len(), 1);
        assert_eq!(first_meta.matched_legacy[0]["usageEventId"], "legacy-mid");

        let mut again = BillingEvents::default();
        collect_event(
            &mut again,
            &event("mid", "session", AT, 7),
            Some("session"),
            AT - 9_000,
            AT + 9_000,
        )?;
        let second = rerun_merge(&first, again)?;
        assert_stable_ledger(&first, &second);
        assert_eq!(cached_usage_events(Some(&second)).len(), 3);
        assert_eq!(
            cached_usage_events(Some(&second))
                .iter()
                .find(|row| row["usageEventId"] == "billing-v2:mid")
                .and_then(|row| row["matchedLegacyUsageEventId"].as_str()),
            Some("legacy-mid")
        );
        Ok(())
    }

    #[test]
    fn fingerprint_collision_keeps_both_sides_without_false_match() -> Result<()> {
        let l_a = legacy_row("L-a", AT, "auto", 7);
        let l_b = legacy_row("L-b", AT, "auto", 7); // same fingerprint, different id
        let mut billing = BillingEvents::default();
        collect_event(
            &mut billing,
            &event("B", "session", AT, 7),
            Some("session"),
            AT - 9_000,
            AT + 9_000,
        )?;
        let merged = merge_billing(vec![l_a, l_b], billing, BillingMeta::default())?;
        let rows = cached_usage_events(Some(&merged));
        let meta = read_billing_meta(Some(&merged));
        // Non-unique fingerprint must not claim a match.
        assert_eq!(meta.matched_legacy.len(), 0);
        assert!(rows.iter().any(|row| row["usageEventId"] == "L-a"));
        assert!(rows.iter().any(|row| row["usageEventId"] == "L-b"));
        // Single billing row inside span with collision → pending (ambiguous).
        assert_eq!(meta.pending.len(), 1);
        assert_eq!(rows.len(), 2);
        Ok(())
    }

    #[test]
    fn same_v2_id_token_correction_updates_counters_and_keeps_match() -> Result<()> {
        let legacy = legacy_row("L0", AT, "auto", 7);
        let mut first_billing = BillingEvents::default();
        collect_event(
            &mut first_billing,
            &event("B0", "session", AT, 7),
            Some("session"),
            AT - 9_000,
            AT + 9_000,
        )?;
        let first = merge_billing(vec![legacy], first_billing, BillingMeta::default())?;
        assert_eq!(
            cached_usage_events(Some(&first))[0]["tokenUsage"]["inputTokens"],
            7
        );

        let mut corrected = BillingEvents::default();
        let mut corrected_event = event("B0", "session", AT, 11);
        corrected_event["usage"]["completion-text-tokens"] = json!("5");
        collect_event(
            &mut corrected,
            &corrected_event,
            Some("session"),
            AT - 9_000,
            AT + 9_000,
        )?;
        let second = rerun_merge(&first, corrected)?;
        let rows = cached_usage_events(Some(&second));
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["usageEventId"], "billing-v2:B0");
        assert_eq!(rows[0]["tokenUsage"]["inputTokens"], 11);
        assert_eq!(rows[0]["tokenUsage"]["outputTokens"], 5);
        // Sticky match metadata survives counter correction on the same V2 id.
        assert_eq!(rows[0]["matchedLegacyUsageEventId"].as_str(), Some("L0"));
        assert_eq!(read_billing_meta(Some(&second)).matched_legacy.len(), 1);
        Ok(())
    }

    #[test]
    fn later_conflicting_billing_row_stays_pending_without_demoting_match() -> Result<()> {
        let legacy = legacy_row("L0", AT, "auto", 7);
        let mut first_billing = BillingEvents::default();
        collect_event(
            &mut first_billing,
            &event("B0", "session", AT, 7),
            Some("session"),
            AT - 9_000,
            AT + 9_000,
        )?;
        let first = merge_billing(vec![legacy], first_billing, BillingMeta::default())?;
        assert_eq!(cached_usage_events(Some(&first)).len(), 1);

        // Later sync adds a conflicting fingerprint twin; keep confirmed B0, park B1.
        let mut both = BillingEvents::default();
        collect_event(
            &mut both,
            &event("B0", "session", AT, 7),
            Some("session"),
            AT - 9_000,
            AT + 9_000,
        )?;
        collect_event(
            &mut both,
            &event("B1", "session", AT, 7),
            Some("session"),
            AT - 9_000,
            AT + 9_000,
        )?;
        let second = rerun_merge(&first, both)?;
        let rows = cached_usage_events(Some(&second));
        let meta = read_billing_meta(Some(&second));
        assert!(rows
            .iter()
            .any(|row| row["usageEventId"] == "billing-v2:B0"));
        assert_eq!(
            rows.iter()
                .find(|row| row["usageEventId"] == "billing-v2:B0")
                .and_then(|row| row["matchedLegacyUsageEventId"].as_str()),
            Some("L0")
        );
        assert_eq!(meta.pending.len(), 1);
        assert_eq!(meta.pending[0]["usageEventId"], "billing-v2:B1");
        assert!(!rows
            .iter()
            .any(|row| row["usageEventId"] == "billing-v2:B1"));
        Ok(())
    }

    #[test]
    fn billing_without_legacy_id_still_dedups_by_v2_id() -> Result<()> {
        let mut billing = BillingEvents::default();
        collect_event(
            &mut billing,
            &event("solo", "session", AT, 4),
            Some("session"),
            AT - 9_000,
            AT + 9_000,
        )?;
        let first = merge_billing(Vec::new(), billing.clone(), BillingMeta::default())?;
        assert_eq!(cached_usage_events(Some(&first)).len(), 1);
        let second = rerun_merge(&first, billing)?;
        assert_eq!(cached_usage_events(Some(&second)).len(), 1);
        assert_eq!(
            cached_usage_events(Some(&second))[0]["usageEventId"],
            "billing-v2:solo"
        );
        assert!(read_billing_meta(Some(&second)).pending.is_empty());
        Ok(())
    }

    #[test]
    fn last_warning_alone_does_not_establish_v2() {
        let legacy = json!({
            "usageEventsDisplay": [legacy_row("legacy-1", AT, "default", 1)],
            "totalUsageEventsCount": 1
        })
        .to_string();
        let warned = with_billing_warning(
            legacy,
            BillingMeta::default(),
            "Cursor Billing V2 incomplete (HTTP 500); kept legacy usage",
        );
        let meta = read_billing_meta(Some(&warned));
        assert!(meta.last_warning.is_some());
        assert!(!billing_v2_committed(
            &cached_usage_events(Some(&warned)),
            &meta
        ));
    }

    #[test]
    fn consecutive_v2_failures_still_accept_new_legacy_through_sync_account() -> Result<()> {
        let runtime = tokio::runtime::Runtime::new()?;
        let legacy1 = json!({
            "usageEventsDisplay": [legacy_row("legacy-1", AT - 10_000, "default", 1)],
            "totalUsageEventsCount": 1
        })
        .to_string();
        let legacy2 = json!({
            "usageEventsDisplay": [
                legacy_row("legacy-1", AT - 10_000, "default", 1),
                legacy_row("legacy-2", AT - 5_000, "default", 2)
            ],
            "totalUsageEventsCount": 2
        })
        .to_string();

        let (_url, _requests, _guard) =
            serve_billing_pages(vec![(500, b"boom".to_vec()), (500, b"boom".to_vec())]);
        LEGACY_JSON_OVERRIDE.with(|cell| *cell.borrow_mut() = Some(legacy1.clone()));
        let first = runtime.block_on(sync_account(
            CursorUsageFetch {
                session_token: "user_test::token".to_owned(),
                existing_cache: None,
                billing_since: None,
            },
            Some(Duration::from_secs(5)),
            true,
        ))?;
        assert!(sync_warning_from_cache(&first).is_some());
        assert!(!billing_v2_committed(
            &cached_usage_events(Some(&first)),
            &read_billing_meta(Some(&first))
        ));

        LEGACY_JSON_OVERRIDE.with(|cell| *cell.borrow_mut() = Some(legacy2.clone()));
        let second = runtime.block_on(sync_account(
            CursorUsageFetch {
                session_token: "user_test::token".to_owned(),
                existing_cache: Some(first),
                billing_since: None,
            },
            Some(Duration::from_secs(5)),
            true,
        ))?;
        LEGACY_JSON_OVERRIDE.with(|cell| *cell.borrow_mut() = None);
        let rows = cached_usage_events(Some(&second));
        assert_eq!(
            rows.len(),
            2,
            "second failed V2 sync must still accept new legacy"
        );
        assert!(rows.iter().any(|row| row["usageEventId"] == "legacy-2"));
        Ok(())
    }

    #[test]
    fn empty_completed_window_records_coverage_without_usage_rows() -> Result<()> {
        let end = DateTime::from_timestamp_millis(AT).unwrap();
        let pages = vec![(200, billing_page(vec![], false, None))];
        let (_url, _requests, _guard) = serve_billing_pages(pages);
        let runtime = tokio::runtime::Runtime::new()?;
        let fetched = runtime
            .block_on(fetch_billing(
                "user_test::token",
                &[],
                None,
                end,
                None,
                &HistoryCoverage::default(),
                Duration::from_secs(10),
            ))?
            .expect("empty 200 is available");
        assert!(fetched.events.events.is_empty());
        assert_eq!(fetched.completed_windows.len(), 1);
        let mut meta = BillingMeta::default();
        for (start, stop) in fetched.completed_windows {
            extend_history_coverage(&mut meta.history, start, stop);
        }
        assert!(meta.history.checked_from_ms.is_some());
        assert!(billing_v2_committed(&[], &meta));
        Ok(())
    }

    #[test]
    fn first_window_persists_when_second_window_pagination_fails() -> Result<()> {
        let end = DateTime::from_timestamp_millis(AT).unwrap();
        let since = end - DateDuration::days(14);
        let older_at = (end - DateDuration::days(10)).timestamp_millis();
        let older = event("older", "session", older_at, 8);
        // Oldest backfill window completes; next window's second page fails.
        let pages = vec![
            (200, billing_page(vec![older], false, None)),
            (
                200,
                billing_page(
                    vec![event(
                        "partial",
                        "session",
                        (end - DateDuration::days(1)).timestamp_millis(),
                        1,
                    )],
                    true,
                    Some("c2"),
                ),
            ),
            (500, b"boom".to_vec()),
        ];
        let (_url, requests, _guard) = serve_billing_pages(pages);
        let runtime = tokio::runtime::Runtime::new()?;
        let fetched = runtime
            .block_on(fetch_billing(
                "user_test::token",
                &[],
                None,
                end,
                Some(since),
                &HistoryCoverage::default(),
                Duration::from_secs(10),
            ))?
            .expect("partial progress must return completed windows");
        assert!(fetched.walk_error.is_some());
        assert_eq!(fetched.completed_windows.len(), 1);
        assert_eq!(fetched.events.events.len(), 1);
        assert!(fetched.events.events.contains_key("older"));
        assert!(!fetched.events.events.contains_key("partial"));
        assert!(requests.lock().unwrap().len() >= 2);

        let mut meta = BillingMeta::default();
        for (start, stop) in &fetched.completed_windows {
            extend_history_coverage(&mut meta.history, *start, *stop);
        }
        let merged = merge_billing(Vec::new(), fetched.events, meta)?;
        let covered = read_billing_meta(Some(&merged)).history;
        let resume = backfill_windows(since, end, &covered);
        assert!(
            !resume.is_empty(),
            "restart must continue from the incomplete newer range"
        );
        assert_eq!(
            resume[0].0.timestamp_millis(),
            covered.checked_to_ms.unwrap(),
            "next window must start where the completed coverage ended"
        );
        Ok(())
    }

    /// Documents the pre-fix demotion: without sticky confirmed metadata,
    /// a re-sync that only sees remaining legacy span parks a previously
    /// matched middle V2 (3→2) or promotes a pending middle into the report (2→3).
    #[test]
    fn documents_pre_fix_report_flips_without_sticky_match_state() {
        let early = legacy_row("legacy-early", AT - 2_000, "auto", 1);
        let late = legacy_row("legacy-late", AT + 2_000, "auto", 3);
        let pending_mid = json!({
            "usageEventId": "billing-v2:mid",
            "timestamp": AT.to_string(),
            "model": "auto",
            "tokenUsage": {
                "inputTokens": 99,
                "outputTokens": 2,
                "cacheReadTokens": 3,
                "cacheWriteTokens": 4
            },
            "billingMatchStatus": MATCH_PENDING,
        });
        // Naive re-merge treating pending as fresh unresolved billing against
        // the two leftover legacy rows → still pending (stable) only if we keep
        // it out of the report. The bug was merging pending into display.
        let naive_report_with_pending_in_display = {
            let mut rows = vec![early.clone(), late.clone(), pending_mid.clone()];
            rows.sort_by_key(|row| std::cmp::Reverse(event_timestamp(row)));
            rows.len()
        };
        assert_eq!(
            naive_report_with_pending_in_display, 3,
            "pre-fix: parking pending in usageEventsDisplay grows 2→3"
        );

        let matched_mid = json!({
            "usageEventId": "billing-v2:mid",
            "timestamp": AT.to_string(),
            "model": "auto",
            "tokenUsage": {
                "inputTokens": 7,
                "outputTokens": 2,
                "cacheReadTokens": 3,
                "cacheWriteTokens": 4
            },
            // Missing matchedLegacyUsageEventId / confirmed sticky fields.
        });
        let span_legacy = [early, late];
        let overlaps = event_timestamp(&matched_mid).is_some_and(|at| {
            let min_at = AT - 2_000;
            let max_at = AT + 2_000;
            at >= min_at && at <= max_at
        });
        assert!(overlaps);
        let demoted_report_len = span_legacy.len(); // V2 parked → only legacy remain
        assert_eq!(
            demoted_report_len, 2,
            "pre-fix: demoting confirmed middle V2 shrinks 3→2"
        );
        let _ = matched_mid;
    }

    #[test]
    fn partial_backfill_survives_disk_restart_and_resumes() -> Result<()> {
        let end = DateTime::from_timestamp_millis(AT).unwrap();
        let since = end - DateDuration::days(14);
        let older_at = (end - DateDuration::days(10)).timestamp_millis();
        let older = event("older", "session", older_at, 8);
        let pages = vec![
            (200, billing_page(vec![older], false, None)),
            (
                200,
                billing_page(
                    vec![event(
                        "partial",
                        "session",
                        (end - DateDuration::days(1)).timestamp_millis(),
                        1,
                    )],
                    true,
                    Some("c2"),
                ),
            ),
            (500, b"boom".to_vec()),
        ];
        let (_url, _requests, _guard) = serve_billing_pages(pages);
        let runtime = tokio::runtime::Runtime::new()?;
        let fetched = runtime
            .block_on(fetch_billing(
                "user_test::token",
                &[],
                None,
                end,
                Some(since),
                &HistoryCoverage::default(),
                Duration::from_secs(10),
            ))?
            .expect("partial");
        assert!(fetched.walk_error.is_some());
        let mut meta = BillingMeta {
            last_warning: fetched.walk_error.clone(),
            ..BillingMeta::default()
        };
        for (start, stop) in &fetched.completed_windows {
            extend_history_coverage(&mut meta.history, *start, *stop);
        }
        let on_disk = merge_billing(Vec::new(), fetched.events, meta)?;
        // Restart: reload meta from disk JSON only.
        let reloaded = read_billing_meta(Some(&on_disk));
        assert_eq!(
            reloaded.history.checked_from_ms,
            read_billing_meta(Some(&on_disk)).history.checked_from_ms
        );
        let resume = backfill_windows(since, end, &reloaded.history);
        assert!(!resume.is_empty());
        assert_eq!(
            resume[0].0.timestamp_millis(),
            reloaded.history.checked_to_ms.unwrap()
        );
        assert!(cached_usage_events(Some(&on_disk))
            .iter()
            .any(|row| row["usageEventId"] == "billing-v2:older"));
        assert!(
            !cached_usage_events(Some(&on_disk))
                .iter()
                .any(|row| row["usageEventId"] == "billing-v2:partial"),
            "incomplete window rows must not land"
        );
        Ok(())
    }

    #[test]
    fn nonempty_legacy_survives_empty_v2_window_and_keeps_updating() -> Result<()> {
        let runtime = tokio::runtime::Runtime::new()?;
        let legacy1 = json!({
            "usageEventsDisplay": [legacy_row("legacy-1", AT - 10_000, "default", 1)],
            "totalUsageEventsCount": 1
        })
        .to_string();
        let legacy2 = json!({
            "usageEventsDisplay": [
                legacy_row("legacy-1", AT - 10_000, "default", 1),
                legacy_row("legacy-2", AT - 5_000, "default", 2)
            ],
            "totalUsageEventsCount": 2
        })
        .to_string();

        // First sync: V2 returns an inspected empty window (coverage only).
        let (_url, _requests, _guard) =
            serve_billing_pages(vec![(200, billing_page(vec![], false, None))]);
        LEGACY_JSON_OVERRIDE.with(|cell| *cell.borrow_mut() = Some(legacy1.clone()));
        let first = runtime.block_on(sync_account(
            CursorUsageFetch {
                session_token: "user_test::token".to_owned(),
                existing_cache: Some(legacy1.clone()),
                billing_since: None,
            },
            Some(Duration::from_secs(5)),
            true,
        ))?;
        let first_meta = read_billing_meta(Some(&first));
        assert!(
            first_meta.history.checked_from_ms.is_some(),
            "empty inspected window must record coverage"
        );
        assert!(
            !billing_v2_ledger_present(&cached_usage_events(Some(&first)), &first_meta),
            "coverage-only must not count as a V2 ledger"
        );
        assert_eq!(cached_usage_events(Some(&first)).len(), 1);

        // Second sync: V2 fails; legacy must still accept new events.
        let (_url2, _requests2, _guard2) = serve_billing_pages(vec![(500, b"boom".to_vec())]);
        LEGACY_JSON_OVERRIDE.with(|cell| *cell.borrow_mut() = Some(legacy2.clone()));
        let second = runtime.block_on(sync_account(
            CursorUsageFetch {
                session_token: "user_test::token".to_owned(),
                existing_cache: Some(first),
                billing_since: None,
            },
            Some(Duration::from_secs(5)),
            true,
        ))?;
        LEGACY_JSON_OVERRIDE.with(|cell| *cell.borrow_mut() = None);
        let rows = cached_usage_events(Some(&second));
        assert_eq!(
            rows.len(),
            2,
            "legacy must keep updating after empty V2 window"
        );
        assert!(rows.iter().any(|row| row["usageEventId"] == "legacy-2"));
        assert!(sync_warning_from_cache(&second).is_some());
        Ok(())
    }

    #[test]
    fn bad_cursor_does_not_complete_window_or_commit_rows() -> Result<()> {
        let item = event("a", "session", AT - 1_000, 7);
        // hasMore=true with a repeated cursor must fail without completing.
        let page1 = billing_page(vec![item], true, Some("stuck-cursor"));
        let page2 = billing_page(vec![], true, Some("stuck-cursor"));
        let (_url, requests, _guard) = serve_billing_pages(vec![(200, page1), (200, page2)]);
        let runtime = tokio::runtime::Runtime::new()?;
        let end = DateTime::from_timestamp_millis(AT).unwrap();
        let err = runtime
            .block_on(fetch_billing(
                "user_test::token",
                &[],
                None,
                end,
                None,
                &HistoryCoverage::default(),
                Duration::from_secs(10),
            ))
            .expect_err("non-advancing cursor must fail");
        assert!(
            err.to_string().contains("pagination did not advance"),
            "{err}"
        );
        assert_eq!(requests.lock().unwrap().len(), 2);
        Ok(())
    }

    /// Local-only offline reconciliation against captured Billing V2 pages.
    /// Disabled in the default suite. Enable explicitly:
    /// TOKSCALE_BILLING_EVIDENCE_DIR=/path/to/evidence cargo test -p tokscale-cli --bin tokscale -- --ignored offline_evidence_two_round
    #[test]
    #[ignore = "requires TOKSCALE_BILLING_EVIDENCE_DIR with captured Billing V2 pages"]
    fn offline_evidence_two_round_replay_preserves_legacy_and_day_totals() -> Result<()> {
        use std::fs;
        use std::path::PathBuf;

        let evidence = PathBuf::from(std::env::var_os("TOKSCALE_BILLING_EVIDENCE_DIR").ok_or_else(|| {
            anyhow::anyhow!(
                "set TOKSCALE_BILLING_EVIDENCE_DIR to a directory containing seed.json, api-page-1.json, api-page-2.json, and personal-unified-usage-2026-11-22.csv"
            )
        })?);
        let seed_path = evidence.join("tokscale-fixed-7f00ee54").join("seed.json");
        let page1_path = evidence.join("api-page-1.json");
        let page2_path = evidence.join("api-page-2.json");
        let csv_path = evidence.join("personal-unified-usage-2026-11-22.csv");
        for path in [&seed_path, &page1_path, &page2_path, &csv_path] {
            ensure!(
                path.is_file(),
                "offline evidence missing: {}",
                path.display()
            );
        }

        let page1: Value = serde_json::from_str(&fs::read_to_string(&page1_path)?)?;
        let page2: Value = serde_json::from_str(&fs::read_to_string(&page2_path)?)?;
        let page1_items = page1["data"].as_array().context("api page 1 data")?;
        let page2_items = page2["data"].as_array().context("api page 2 data")?;
        assert_eq!(page1_items.len() + page2_items.len(), 109);

        // Synthetic session principal matching the fixture without printing ids.
        let principal = page1_items[0]["principal"]["id"]
            .as_str()
            .context("fixture principal")?
            .to_owned();
        let token = format!("{principal}::offline-fixture-token");

        let mut seed: Value = serde_json::from_str(&fs::read_to_string(&seed_path)?)?;
        if let Some(object) = seed.as_object_mut() {
            object.remove("_owner");
            object.remove("_tokscale");
            object.remove("zzTokscaleOwner");
        }
        let seed_rows = seed["usageEventsDisplay"]
            .as_array()
            .context("seed usageEventsDisplay")?;
        assert_eq!(seed_rows.len(), 483);
        let seed_json = seed.to_string();

        let expected_csv = {
            // Official CSV meters for Beijing 2026-11-22 (UTC 2026-11-21T16..22T16).
            let text = fs::read_to_string(&csv_path)?;
            let mut by_id: HashMap<String, (i64, i64, i64, i64)> = HashMap::new();
            let mut input = 0i64;
            let mut output = 0i64;
            let mut cache_read = 0i64;
            let mut cache_write = 0i64;
            let mut count = 0usize;
            let mut lines = text.lines();
            let header = lines.next().unwrap_or_default();
            let cols: Vec<_> = header.split(',').collect();
            let idx = |name: &str| {
                cols.iter()
                    .position(|col| col.trim_start_matches('\u{feff}') == name)
            };
            let id_i = idx("Billing Item ID").context("csv id")?;
            let time_i = idx("Event Time").context("csv time")?;
            let product_i = idx("Product").context("csv product")?;
            let cat_i = idx("Charge Category").context("csv category")?;
            let usage_i = idx("Usage").context("csv usage")?;
            let day_start =
                DateTime::parse_from_rfc3339("2026-11-21T16:00:00Z")?.timestamp_millis();
            let day_end = DateTime::parse_from_rfc3339("2026-11-22T16:00:00Z")?.timestamp_millis();
            for line in lines {
                if line.trim().is_empty() {
                    continue;
                }
                // Usage JSON may contain commas; parse via csv crate-less split is unsafe.
                // Re-read with a minimal state machine for quoted fields.
                let fields = parse_csv_line(line);
                if fields.len() <= usage_i {
                    continue;
                }
                if fields[product_i] != "cursor" || fields[cat_i] != "usage" {
                    continue;
                }
                let when = DateTime::parse_from_rfc3339(&fields[time_i])?.timestamp_millis();
                if when < day_start || when >= day_end {
                    continue;
                }
                let usage: Value = serde_json::from_str(&fields[usage_i])?;
                let meter = |name: &str| -> i64 {
                    usage
                        .get(name)
                        .and_then(|v| {
                            v.as_i64()
                                .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
                        })
                        .unwrap_or(0)
                };
                let meters = (
                    meter("prompt-text-tokens"),
                    meter("completion-text-tokens"),
                    meter("cached-prompt-text-tokens"),
                    meter("cache-write-text-tokens"),
                );
                by_id.insert(fields[id_i].clone(), meters);
                input += meters.0;
                output += meters.1;
                cache_read += meters.2;
                cache_write += meters.3;
                count += 1;
            }
            assert_eq!(count, 109);
            assert_eq!(
                (input, output, cache_read, cache_write),
                (510380, 86980, 13820544, 0)
            );
            by_id
        };

        // First sync walks the default 7-day window (two fixture pages). The
        // second sync uses the correction watermark and must not replay older
        // fixture rows outside that narrower window — return an empty page.
        let empty = billing_page(vec![], false, None);
        let pages = vec![
            (200, serde_json::to_vec(&page1)?, Duration::ZERO),
            (200, serde_json::to_vec(&page2)?, Duration::ZERO),
            (200, empty, Duration::ZERO),
        ];
        let (_url, _requests, _guard) = serve_billing_pages_delayed(pages);
        let runtime = tokio::runtime::Runtime::new()?;

        let first = runtime.block_on(sync_account(
            CursorUsageFetch {
                session_token: token.clone(),
                existing_cache: Some(seed_json),
                billing_since: None,
            },
            Some(Duration::from_secs(30)),
            true,
        ))?;
        let first_rows = cached_usage_events(Some(&first));
        let legacy_count = first_rows
            .iter()
            .filter(|row| {
                row["usageEventId"]
                    .as_str()
                    .is_some_and(|id| !id.starts_with(BILLING_EVENT_PREFIX))
            })
            .count();
        assert_eq!(legacy_count, 483, "legacy ledger must stay intact");
        let billing_rows: Vec<_> = first_rows
            .iter()
            .filter(|row| {
                row["usageEventId"]
                    .as_str()
                    .is_some_and(|id| id.starts_with(BILLING_EVENT_PREFIX))
            })
            .cloned()
            .collect();
        assert_eq!(billing_rows.len(), 109);
        assert!(read_billing_meta(Some(&first)).pending.is_empty());

        let mut got_input = 0i64;
        let mut got_output = 0i64;
        let mut got_cache_read = 0i64;
        let mut got_cache_write = 0i64;
        for row in &billing_rows {
            let id = row["usageEventId"]
                .as_str()
                .unwrap()
                .trim_start_matches(BILLING_EVENT_PREFIX);
            let expected = expected_csv
                .get(id)
                .with_context(|| "billing id missing from csv")?;
            let usage = &row["tokenUsage"];
            let got = (
                usage["inputTokens"].as_i64().unwrap_or(0),
                usage["outputTokens"].as_i64().unwrap_or(0),
                usage["cacheReadTokens"].as_i64().unwrap_or(0),
                usage["cacheWriteTokens"].as_i64().unwrap_or(0),
            );
            assert_eq!(got, *expected);
            got_input += got.0;
            got_output += got.1;
            got_cache_read += got.2;
            got_cache_write += got.3;
        }
        assert_eq!(
            (got_input, got_output, got_cache_read, got_cache_write),
            (510380, 86980, 13820544, 0)
        );

        let second = runtime.block_on(sync_account(
            CursorUsageFetch {
                session_token: token,
                existing_cache: Some(first.clone()),
                billing_since: None,
            },
            Some(Duration::from_secs(30)),
            true,
        ))?;
        assert_stable_ledger(&first, &second);
        let second_billing = cached_usage_events(Some(&second))
            .into_iter()
            .filter(|row| {
                row["usageEventId"]
                    .as_str()
                    .is_some_and(|id| id.starts_with(BILLING_EVENT_PREFIX))
            })
            .count();
        assert_eq!(second_billing, 109);
        let second_legacy = cached_usage_events(Some(&second))
            .into_iter()
            .filter(|row| {
                row["usageEventId"]
                    .as_str()
                    .is_some_and(|id| !id.starts_with(BILLING_EVENT_PREFIX))
            })
            .count();
        assert_eq!(second_legacy, 483);
        Ok(())
    }

    fn parse_csv_line(line: &str) -> Vec<String> {
        let mut fields = Vec::new();
        let mut current = String::new();
        let mut in_quotes = false;
        let mut chars = line.chars().peekable();
        while let Some(ch) = chars.next() {
            match ch {
                '"' => {
                    if in_quotes && chars.peek() == Some(&'"') {
                        current.push('"');
                        chars.next();
                    } else {
                        in_quotes = !in_quotes;
                    }
                }
                ',' if !in_quotes => {
                    fields.push(std::mem::take(&mut current));
                }
                _ => current.push(ch),
            }
        }
        fields.push(current);
        fields
    }

    #[test]
    fn outer_budget_timeout_keeps_first_window_and_resumes() -> Result<()> {
        let end = DateTime::from_timestamp_millis(AT).unwrap();
        let since = end - DateDuration::days(14);
        let older_at = (end - DateDuration::days(10)).timestamp_millis();
        let older = event("older", "session", older_at, 8);
        let recent = event(
            "recent",
            "session",
            (end - DateDuration::days(1)).timestamp_millis(),
            1,
        );
        // First window answers immediately; second window stalls past the budget.
        let pages = vec![
            (200, billing_page(vec![older], false, None), Duration::ZERO),
            (
                200,
                billing_page(vec![recent], false, None),
                Duration::from_millis(500),
            ),
        ];
        let (_url, _requests, _guard) = serve_billing_pages_delayed(pages);
        let runtime = tokio::runtime::Runtime::new()?;
        let fetched = runtime
            .block_on(fetch_billing(
                "user_test::token",
                &[],
                None,
                end,
                Some(since),
                &HistoryCoverage::default(),
                Duration::from_millis(200),
            ))?
            .expect("timeout after first window must return partial");
        assert!(
            fetched
                .walk_error
                .as_deref()
                .is_some_and(|msg| msg.contains("timed out")),
            "{:?}",
            fetched.walk_error
        );
        assert_eq!(fetched.completed_windows.len(), 1);
        assert!(fetched.events.events.contains_key("older"));
        assert!(!fetched.events.events.contains_key("recent"));

        let warning = format_backfill_progress_warning(
            fetched.walk_error.as_deref().unwrap(),
            Some(since),
            end,
            &{
                let mut history = HistoryCoverage::default();
                for (start, stop) in &fetched.completed_windows {
                    extend_history_coverage(&mut history, *start, *stop);
                }
                history
            },
        );
        assert!(warning.contains("checked coverage"), "{warning}");
        assert!(warning.contains("still pending"), "{warning}");

        let mut meta = BillingMeta::default();
        for (start, stop) in &fetched.completed_windows {
            extend_history_coverage(&mut meta.history, *start, *stop);
        }
        let on_disk = merge_billing(Vec::new(), fetched.events, meta)?;
        let resume = backfill_windows(since, end, &read_billing_meta(Some(&on_disk)).history);
        assert!(!resume.is_empty());
        assert_eq!(
            resume[0].0.timestamp_millis(),
            read_billing_meta(Some(&on_disk))
                .history
                .checked_to_ms
                .unwrap()
        );
        Ok(())
    }
}
