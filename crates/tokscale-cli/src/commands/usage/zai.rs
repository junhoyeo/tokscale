use anyhow::Result;
use chrono::{TimeZone, Utc};
use serde::Deserialize;

use super::helpers::capitalize;
use super::{UsageMetric, UsageOutput};

#[derive(Debug, Deserialize)]
struct QuotaResp {
    data: Option<QuotaData>,
}

#[derive(Debug, Deserialize)]
struct QuotaData {
    limits: Option<Vec<Limit>>,
    level: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Limit {
    #[serde(rename = "type")]
    limit_type: Option<String>,
    #[allow(dead_code)]
    usage: Option<f64>,
    remaining: Option<f64>,
    percentage: Option<f64>,
    #[allow(dead_code)]
    current_value: Option<f64>,
    number: Option<i64>,
    unit: Option<i64>,
    #[serde(alias = "next_reset_time", alias = "nextResetTime")]
    next_reset_time: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SubResp {
    data: Option<Vec<Sub>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Sub {
    #[serde(alias = "product_name", alias = "productName")]
    product_name: Option<String>,
    #[serde(alias = "next_renew_time", alias = "nextRenewTime")]
    next_renew_time: Option<String>,
}

/// Z.ai does not document whether its epoch fields are seconds or
/// milliseconds, so treat anything past the year-2286 second boundary as
/// already being milliseconds. Shared by both entry points below: when only one
/// of them carried the heuristic, a numeric string and a JSON number could
/// resolve to different instants.
///
/// A non-positive epoch is absent, not 1970. `null` is one way to say "no reset
/// scheduled" and a zero epoch is the other, because a serializer whose field
/// is not nullable emits the type's default. Left alone, 0 resolves to
/// `1970-01-01T00:00:00+00:00`, which is a perfectly truthy `Some` everywhere
/// `resets_at` is consumed: it outranks a real reset time carried by a sibling
/// quota entry in `build_metrics`, it blocks the subscription-renewal fallback
/// on the `TIME_LIMIT` path, and `helpers::format_reset_time` renders any past
/// instant as "resets now", so the screen would say that forever.
fn epoch_to_rfc3339(ts: i64) -> Option<String> {
    if ts <= 0 {
        return None;
    }
    // The guard above is load-bearing beyond the sentinel: it is what makes the
    // arithmetic here total. `ts` is positive, so the `* 1000` branch tops out
    // at 10_000_000_000_000, and the negative saturation of the float spelling
    // below can no longer reach an overflowing `abs`.
    let ms = if ts > 10_000_000_000 { ts } else { ts * 1000 };
    Utc.timestamp_millis_opt(ms)
        .single()
        .map(|dt| dt.to_rfc3339())
}

/// Parse one of the timestamp shapes Z.ai sends, or `None`.
///
/// Unrecognized input must not be passed through as if it were a timestamp.
/// `resets_at` is consumed as one: a raw string like `"unknown"` outranks a
/// real reset time carried by a sibling quota entry in `build_metrics`, and it
/// blocks the subscription-renewal fallback on the `TIME_LIMIT` path. It would
/// then reach the screen verbatim, because `helpers::format_reset_time` echoes
/// whatever it cannot parse as RFC 3339.
fn parse_reset_time_str(s: &str) -> Option<String> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    if chrono::DateTime::parse_from_rfc3339(s).is_ok() {
        return Some(s.to_string());
    }
    if let Ok(ts) = s.parse::<i64>() {
        return epoch_to_rfc3339(ts);
    }
    // The float spelling, so a string and a JSON number carrying the same
    // value resolve the same way: `"1788701854998.0"` must agree with
    // `1788701854998.0`, which `parse_reset_time` already accepts. Same
    // rounding, same saturating cast, same range check downstream.
    if let Some(ts) = s
        .parse::<f64>()
        .ok()
        .filter(|f| f.is_finite())
        .map(|f| f.round() as i64)
    {
        return epoch_to_rfc3339(ts);
    }
    if let Ok(d) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        if let Some(dt) = d.and_hms_opt(0, 0, 0) {
            return Some(chrono::DateTime::<Utc>::from_naive_utc_and_offset(dt, Utc).to_rfc3339());
        }
    }
    // Offsetless date-time, with either separator. The `T` spelling is the near
    // miss most likely to arrive from an API that almost emits RFC 3339, and
    // dropping unrecognized input above means it would otherwise be lost.
    for fmt in ["%Y-%m-%d %H:%M:%S", "%Y-%m-%dT%H:%M:%S"] {
        if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, fmt) {
            return Some(chrono::DateTime::<Utc>::from_naive_utc_and_offset(dt, Utc).to_rfc3339());
        }
    }
    None
}

fn parse_reset_time(val: Option<&serde_json::Value>) -> Option<String> {
    let val = val?;
    match val {
        serde_json::Value::Number(n) => {
            // JSON draws no int/float distinction, so whether a millisecond
            // epoch arrives as 1788701854998 or 1788701854998.0 is a detail of
            // Z.ai's serializer. `as_i64` returns None for the float spelling,
            // which would discard the reset time entirely. The cast saturates
            // rather than wrapping, so a float too large to be an epoch lands
            // on an i64 bound and is rejected as out of range.
            let ts = n
                .as_i64()
                .or_else(|| n.as_f64().map(|f| f.round() as i64))?;
            epoch_to_rfc3339(ts)
        }
        serde_json::Value::String(s) => parse_reset_time_str(s),
        _ => None,
    }
}

async fn fetch_quota(client: &reqwest::Client, key: &str) -> Result<QuotaResp> {
    let resp = client
        .get("https://api.z.ai/api/monitor/usage/quota/limit")
        .header("Authorization", format!("Bearer {key}"))
        .header("Accept", "application/json")
        .send()
        .await?;
    if !resp.status().is_success() {
        anyhow::bail!("Z.ai quota request failed (HTTP {})", resp.status());
    }
    Ok(resp.json().await?)
}

async fn fetch_sub(client: &reqwest::Client, key: &str) -> Result<SubResp> {
    let resp = client
        .get("https://api.z.ai/api/biz/subscription/list")
        .header("Authorization", format!("Bearer {key}"))
        .header("Accept", "application/json")
        .send()
        .await?;
    if !resp.status().is_success() {
        anyhow::bail!("Z.ai subscription request failed (HTTP {})", resp.status());
    }
    Ok(resp.json().await?)
}

pub fn has_credentials() -> bool {
    std::env::var("ZAI_API_KEY")
        .or_else(|_| std::env::var("GLM_API_KEY"))
        .is_ok()
}

/// Translate Z.ai's limit windows into the session/weekly/web-search metrics
/// tokscale surfaces.
///
/// Z.ai encodes each window as an opaque `(unit, number)` code pair rather than
/// a name: `(3, 5)` is the 5-hour rolling session window and `(6, 1)` is the
/// 1-week window. Unrecognized codes are skipped rather than guessed at.
fn build_metrics(
    limits: &[Limit],
    search_reset: Option<String>,
    session_metric: &mut Option<UsageMetric>,
    weekly_metric: &mut Option<UsageMetric>,
    search_metric: &mut Option<UsageMetric>,
) {
    for limit in limits.iter() {
        // Skip limits with no percentage rather than fabricating
        // "0% used / 100% left" from a missing field.
        let pct = match limit.percentage {
            Some(p) => p.clamp(0.0, 100.0),
            None => continue,
        };

        match limit.limit_type.as_deref() {
            // V3 GLM Coding plans report the same (unit, number)
            // windows as CREDIT_LIMIT instead of TOKENS_LIMIT. Prefer
            // CREDIT_LIMIT so a plan that ever emits both for one
            // window cannot silently last-write-wins.
            Some("TOKENS_LIMIT") | Some("CREDIT_LIMIT") => {
                let prefer = limit.limit_type.as_deref() == Some("CREDIT_LIMIT");
                let (target, label) = match (limit.unit, limit.number) {
                    (Some(3), Some(5)) => (&mut *session_metric, "Session"),
                    (Some(6), Some(1)) => (&mut *weekly_metric, "Weekly"),
                    _ => continue,
                };
                let resets_at = parse_reset_time(limit.next_reset_time.as_ref());
                if target.is_none() || prefer {
                    let existing_resets_at = target.as_ref().and_then(|m| m.resets_at.clone());
                    *target = Some(UsageMetric {
                        label: label.to_string(),
                        used_percent: pct,
                        remaining_percent: 100.0 - pct,
                        remaining_label: None,
                        resets_at: resets_at.or(existing_resets_at),
                    });
                } else if let Some(ref mut metric) = target {
                    if metric.resets_at.is_none() && resets_at.is_some() {
                        metric.resets_at = resets_at;
                    }
                }
            }
            Some("TIME_LIMIT") => {
                let remaining_label = limit.remaining.map(|r| format!("{:.0} left", r));
                let resets_at = parse_reset_time(limit.next_reset_time.as_ref())
                    .or_else(|| search_reset.clone());
                *search_metric = Some(UsageMetric {
                    label: "Web Search".into(),
                    used_percent: pct,
                    remaining_percent: 100.0 - pct,
                    remaining_label,
                    resets_at,
                });
            }
            _ => {}
        }
    }
}

pub fn fetch() -> Result<UsageOutput> {
    let api_key = std::env::var("ZAI_API_KEY")
        .or_else(|_| std::env::var("GLM_API_KEY"))
        .map_err(|_| anyhow::anyhow!("No ZAI_API_KEY or GLM_API_KEY set."))?;

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(async {
        let client = tokscale_core::http::client();
        let quota = fetch_quota(&client, &api_key).await?;
        let sub = fetch_sub(&client, &api_key).await.ok();

        let base_plan = sub
            .as_ref()
            .and_then(|s| s.data.as_ref())
            .and_then(|d| d.first())
            .and_then(|s| s.product_name.clone())
            .or_else(|| {
                quota
                    .data
                    .as_ref()
                    .and_then(|d| d.level.clone())
                    .map(|l| capitalize(&l))
            });

        let next_renew = sub
            .as_ref()
            .and_then(|s| s.data.as_ref())
            .and_then(|d| d.first())
            .and_then(|s| s.next_renew_time.clone());

        let plan = match (base_plan, next_renew.as_deref()) {
            (Some(tier), Some(renew)) if !renew.trim().is_empty() => {
                Some(format!("{tier} (renews {})", renew.trim()))
            }
            (Some(tier), _) => Some(tier),
            (None, Some(renew)) if !renew.trim().is_empty() => {
                Some(format!("Renews {}", renew.trim()))
            }
            (None, _) => None,
        };

        let mut session_metric = None;
        let mut weekly_metric = None;
        let mut search_metric = None;

        let search_reset = next_renew.as_deref().and_then(parse_reset_time_str);

        if let Some(limits) = quota.data.as_ref().and_then(|d| d.limits.as_ref()) {
            build_metrics(
                limits,
                search_reset,
                &mut session_metric,
                &mut weekly_metric,
                &mut search_metric,
            );
        }

        let mut metrics = Vec::new();
        if let Some(m) = session_metric {
            metrics.push(m);
        }
        if let Some(m) = weekly_metric {
            metrics.push(m);
        }
        if let Some(m) = search_metric {
            metrics.push(m);
        }

        Ok(UsageOutput {
            provider: "Z.ai".into(),
            account: None,
            credential_source: None,
            plan,
            email: None,
            metrics,
            reset_credits: None,
            credit_status: None,
            spend_control: None,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limit(kind: &str, unit: i64, number: i64, percentage: f64) -> Limit {
        serde_json::from_value(serde_json::json!({
            "type": kind,
            "unit": unit,
            "number": number,
            "percentage": percentage,
        }))
        .expect("valid Limit fixture")
    }

    fn limit_with_reset(
        kind: &str,
        unit: i64,
        number: i64,
        percentage: f64,
        reset: serde_json::Value,
    ) -> Limit {
        serde_json::from_value(serde_json::json!({
            "type": kind,
            "unit": unit,
            "number": number,
            "percentage": percentage,
            "nextResetTime": reset,
        }))
        .expect("valid Limit fixture")
    }

    fn run(
        limits: &[Limit],
    ) -> (
        Option<UsageMetric>,
        Option<UsageMetric>,
        Option<UsageMetric>,
    ) {
        run_with_search_reset(limits, None)
    }

    fn run_with_search_reset(
        limits: &[Limit],
        search_reset: Option<String>,
    ) -> (
        Option<UsageMetric>,
        Option<UsageMetric>,
        Option<UsageMetric>,
    ) {
        let mut session = None;
        let mut weekly = None;
        let mut search = None;
        build_metrics(limits, search_reset, &mut session, &mut weekly, &mut search);
        (session, weekly, search)
    }

    #[test]
    fn credit_limit_session_window_maps_to_session_metric() {
        let (session, weekly, _) = run(&[limit("CREDIT_LIMIT", 3, 5, 40.0)]);
        let session = session.expect("session metric present");
        assert_eq!(session.label, "Session");
        assert_eq!(session.used_percent, 40.0);
        assert_eq!(session.remaining_percent, 60.0);
        assert!(weekly.is_none());
    }

    #[test]
    fn credit_limit_weekly_window_maps_to_weekly_metric() {
        let (session, weekly, _) = run(&[limit("CREDIT_LIMIT", 6, 1, 25.0)]);
        let weekly = weekly.expect("weekly metric present");
        assert_eq!(weekly.label, "Weekly");
        assert_eq!(weekly.used_percent, 25.0);
        assert!(session.is_none());
    }

    #[test]
    fn credit_limit_is_preferred_over_tokens_limit_for_same_window() {
        // TOKENS_LIMIT first, then CREDIT_LIMIT: credit must win.
        let (session, _, _) = run(&[
            limit("TOKENS_LIMIT", 3, 5, 10.0),
            limit("CREDIT_LIMIT", 3, 5, 90.0),
        ]);
        assert_eq!(session.expect("session metric present").used_percent, 90.0);

        // Reversed order: CREDIT_LIMIT must not be clobbered by a later
        // TOKENS_LIMIT for the same window.
        let (session, _, _) = run(&[
            limit("CREDIT_LIMIT", 3, 5, 90.0),
            limit("TOKENS_LIMIT", 3, 5, 10.0),
        ]);
        assert_eq!(session.expect("session metric present").used_percent, 90.0);
    }

    #[test]
    fn unrecognized_window_codes_are_skipped() {
        let (session, weekly, search) = run(&[limit("CREDIT_LIMIT", 9, 9, 50.0)]);
        assert!(session.is_none());
        assert!(weekly.is_none());
        assert!(search.is_none());
    }

    #[test]
    fn credit_limit_with_next_reset_time_sets_resets_at() {
        let lim: Limit = serde_json::from_value(serde_json::json!({
            "type": "CREDIT_LIMIT",
            "unit": 6,
            "number": 1,
            "percentage": 100.0,
            "nextResetTime": 1788701854998i64,
        }))
        .expect("valid limit with nextResetTime");

        let (_, weekly, _) = run(&[lim]);
        let weekly = weekly.expect("weekly metric present");
        assert_eq!(weekly.used_percent, 100.0);
        assert_eq!(weekly.remaining_percent, 0.0);
        assert_eq!(
            weekly.resets_at.as_deref(),
            Some("2026-09-06T13:37:34.998+00:00")
        );
    }

    #[test]
    fn credit_limit_preserves_resets_at_from_tokens_limit_fallback() {
        let tokens_lim: Limit = serde_json::from_value(serde_json::json!({
            "type": "TOKENS_LIMIT",
            "unit": 3,
            "number": 5,
            "percentage": 20.0,
            "nextResetTime": 1788701854998i64,
        }))
        .expect("tokens limit");
        let credit_lim: Limit = serde_json::from_value(serde_json::json!({
            "type": "CREDIT_LIMIT",
            "unit": 3,
            "number": 5,
            "percentage": 80.0,
        }))
        .expect("credit limit without reset");

        let (session, _, _) = run(&[tokens_lim, credit_lim]);
        let session = session.expect("session metric present");
        assert_eq!(session.used_percent, 80.0);
        assert_eq!(
            session.resets_at.as_deref(),
            Some("2026-09-06T13:37:34.998+00:00")
        );
    }

    #[test]
    fn sub_resp_deserializes_camel_case_fields() {
        let json = serde_json::json!({
            "data": [{
                "id": "845544",
                "productName": "GLM Coding Max",
                "nextRenewTime": "2026-11-30"
            }]
        });
        let sub_resp: SubResp = serde_json::from_value(json).expect("valid SubResp");
        let sub = sub_resp.data.as_ref().unwrap().first().unwrap();
        assert_eq!(sub.product_name.as_deref(), Some("GLM Coding Max"));
        assert_eq!(sub.next_renew_time.as_deref(), Some("2026-11-30"));
    }

    #[test]
    fn parse_reset_time_handles_dates_and_epoch() {
        let val_num = serde_json::json!(1788701854998i64);
        assert_eq!(
            parse_reset_time(Some(&val_num)).as_deref(),
            Some("2026-09-06T13:37:34.998+00:00")
        );

        let val_str = serde_json::json!("1788701854998");
        assert_eq!(
            parse_reset_time(Some(&val_str)).as_deref(),
            Some("2026-09-06T13:37:34.998+00:00")
        );

        let val_date = serde_json::json!("2026-11-30");
        assert_eq!(
            parse_reset_time(Some(&val_date)).as_deref(),
            Some("2026-11-30T00:00:00+00:00")
        );
    }

    /// cubic on e5ca8b5b: the numeric-string path accepted only an integer
    /// spelling, so `"1788701854998.0"` resolved to `None` while the same
    /// value as a JSON number resolved to a reset time. Both entry points
    /// must agree, or the unification `epoch_to_rfc3339` exists for is
    /// only half true.
    #[test]
    fn float_epoch_string_agrees_with_float_epoch_number() {
        let as_number = parse_reset_time(Some(&serde_json::json!(1788701854998.0)));
        let as_string = parse_reset_time(Some(&serde_json::json!("1788701854998.0")));
        assert_eq!(as_number.as_deref(), Some("2026-09-06T13:37:34.998+00:00"));
        assert_eq!(as_string, as_number);
        // seconds-precision float too, and the integer spellings still agree
        let secs_num = parse_reset_time(Some(&serde_json::json!(1788701854.0)));
        let secs_str = parse_reset_time(Some(&serde_json::json!("1788701854.0")));
        assert_eq!(secs_str, secs_num);
        assert_eq!(secs_num.as_deref(), Some("2026-09-06T13:37:34+00:00"));
        // non-finite spellings are not epochs
        for bad in ["inf", "-inf", "nan", "NaN"] {
            assert_eq!(parse_reset_time_str(bad), None, "{bad}");
        }
    }

    #[test]
    fn float_epoch_is_not_dropped() {
        // JSON draws no int/float distinction, so the same millisecond epoch
        // can arrive either way depending on Z.ai's serializer.
        let as_int = serde_json::json!(1788701854998i64);
        let as_float = serde_json::json!(1788701854998.0f64);
        assert_eq!(
            parse_reset_time(Some(&as_float)).as_deref(),
            Some("2026-09-06T13:37:34.998+00:00")
        );
        assert_eq!(
            parse_reset_time(Some(&as_float)),
            parse_reset_time(Some(&as_int))
        );
    }

    #[test]
    fn second_and_millisecond_epochs_agree_across_json_types() {
        let secs = serde_json::json!(1788701854i64);
        let secs_float = serde_json::json!(1788701854.0f64);
        let secs_str = serde_json::json!("1788701854");
        assert_eq!(
            parse_reset_time(Some(&secs)).as_deref(),
            Some("2026-09-06T13:37:34+00:00")
        );
        assert_eq!(
            parse_reset_time(Some(&secs_str)),
            parse_reset_time(Some(&secs))
        );
        assert_eq!(
            parse_reset_time(Some(&secs_float)),
            parse_reset_time(Some(&secs))
        );
    }

    #[test]
    fn out_of_range_epochs_are_rejected_without_panicking() {
        // Floats saturate on cast rather than wrapping, so these land on the
        // i64 bounds; the numeric-string spelling reaches i64::MIN directly.
        // The two negatives are now turned away by the sign guard and the
        // positive one by chrono's range check, so this stays a regression
        // test against either half being relaxed.
        assert_eq!(parse_reset_time(Some(&serde_json::json!(-1e300f64))), None);
        assert_eq!(parse_reset_time(Some(&serde_json::json!(1e300f64))), None);
        assert_eq!(
            parse_reset_time(Some(&serde_json::json!("-9223372036854775808"))),
            None
        );
    }

    #[test]
    fn offsetless_date_time_is_accepted_with_either_separator() {
        // Rejecting unrecognized input means a near-miss RFC 3339 value would
        // otherwise be dropped instead of merely rendering badly.
        assert_eq!(
            parse_reset_time(Some(&serde_json::json!("2026-11-30T04:05:06"))).as_deref(),
            Some("2026-11-30T04:05:06+00:00")
        );
        assert_eq!(
            parse_reset_time(Some(&serde_json::json!("2026-11-30 04:05:06"))).as_deref(),
            Some("2026-11-30T04:05:06+00:00")
        );
    }

    #[test]
    fn unrecognized_reset_time_is_rejected() {
        let bogus = serde_json::json!("unknown");
        assert_eq!(parse_reset_time(Some(&bogus)), None);
    }

    #[test]
    fn unparseable_reset_time_never_suppresses_a_valid_one() {
        // An unparseable value must not be treated as a timestamp, in either
        // arrival order: as a raw string it would win both the CREDIT_LIMIT
        // overwrite and the `resets_at.is_none()` fill-in below.
        let valid = || {
            limit_with_reset(
                "TOKENS_LIMIT",
                3,
                5,
                20.0,
                serde_json::json!(1788701854998i64),
            )
        };
        let bogus = || limit_with_reset("CREDIT_LIMIT", 3, 5, 80.0, serde_json::json!("unknown"));

        let (session, _, _) = run(&[valid(), bogus()]);
        let session = session.expect("session metric present");
        assert_eq!(session.used_percent, 80.0);
        assert_eq!(
            session.resets_at.as_deref(),
            Some("2026-09-06T13:37:34.998+00:00")
        );

        let (session, _, _) = run(&[bogus(), valid()]);
        let session = session.expect("session metric present");
        assert_eq!(session.used_percent, 80.0);
        assert_eq!(
            session.resets_at.as_deref(),
            Some("2026-09-06T13:37:34.998+00:00")
        );
    }

    #[test]
    fn time_limit_falls_back_to_renewal_when_its_own_reset_is_unparseable() {
        let lim = limit_with_reset("TIME_LIMIT", 0, 0, 10.0, serde_json::json!("unknown"));
        let (_, _, search) =
            run_with_search_reset(&[lim], Some("2026-11-30T00:00:00+00:00".to_string()));
        let search = search.expect("web search metric present");
        assert_eq!(
            search.resets_at.as_deref(),
            Some("2026-11-30T00:00:00+00:00")
        );
    }

    #[test]
    fn non_positive_epochs_are_rejected_in_every_spelling() {
        // Epoch 0 is a sentinel, not a reset time. Left alone it resolves to
        // 1970-01-01, which is truthy everywhere `resets_at` is consumed.
        for zero in [
            serde_json::json!(0i64),
            serde_json::json!(0.0f64),
            serde_json::json!("0"),
        ] {
            assert_eq!(parse_reset_time(Some(&zero)), None, "spelling: {zero}");
        }
        // A reset time before the epoch is nonsense for a *next* reset.
        for negative in [
            serde_json::json!(-1i64),
            serde_json::json!(-1.0f64),
            serde_json::json!("-1"),
        ] {
            assert_eq!(
                parse_reset_time(Some(&negative)),
                None,
                "spelling: {negative}"
            );
        }
    }

    #[test]
    fn zero_epoch_never_suppresses_a_valid_reset_time() {
        // Same defect class as the unparseable-string case above, reached
        // through the numeric path: as 1970-01-01 the sentinel wins both the
        // CREDIT_LIMIT overwrite and the `resets_at.is_none()` fill-in, and
        // `helpers::format_reset_time` then renders "resets now" forever.
        let valid = || {
            limit_with_reset(
                "TOKENS_LIMIT",
                3,
                5,
                20.0,
                serde_json::json!(1788701854998i64),
            )
        };
        let sentinel = || limit_with_reset("CREDIT_LIMIT", 3, 5, 80.0, serde_json::json!(0i64));

        let (session, _, _) = run(&[valid(), sentinel()]);
        let session = session.expect("session metric present");
        assert_eq!(session.used_percent, 80.0);
        assert_eq!(
            session.resets_at.as_deref(),
            Some("2026-09-06T13:37:34.998+00:00")
        );

        let (session, _, _) = run(&[sentinel(), valid()]);
        let session = session.expect("session metric present");
        assert_eq!(session.used_percent, 80.0);
        assert_eq!(
            session.resets_at.as_deref(),
            Some("2026-09-06T13:37:34.998+00:00")
        );
    }

    #[test]
    fn time_limit_falls_back_to_renewal_when_its_own_reset_is_zero() {
        let lim = limit_with_reset("TIME_LIMIT", 0, 0, 10.0, serde_json::json!(0i64));
        let (_, _, search) =
            run_with_search_reset(&[lim], Some("2026-11-30T00:00:00+00:00".to_string()));
        let search = search.expect("web search metric present");
        assert_eq!(
            search.resets_at.as_deref(),
            Some("2026-11-30T00:00:00+00:00")
        );
    }
}
