//! Amp subscription usage card.
//!
//! The installed `amp` CLI owns Amp credentials, and their on-disk format has
//! changed across releases, so `amp usage` is tried first. When the CLI is
//! missing or fails, an API key from `AMP_API_KEY` or the legacy
//! `secrets.json` is sent to the same balance endpoint the CLI uses. Both
//! paths return the same human-readable balance text, parsed below.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::PathBuf;
use std::time::Duration;

use super::{UsageMetric, UsageOutput};

const AMP_USAGE_CLI_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Deserialize)]
struct Secrets {
    #[serde(rename = "apiKey@https://ampcode.com/")]
    api_key: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ApiResponse {
    ok: Option<bool>,
    result: Option<ApiResult>,
    error: Option<ApiError>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApiResult {
    display_text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ApiError {
    code: Option<String>,
    message: Option<String>,
}

fn secrets_path() -> Option<PathBuf> {
    crate::paths::home_dir().map(|home| home.join(".local/share/amp/secrets.json"))
}

fn api_key_from_env() -> Option<String> {
    std::env::var("AMP_API_KEY")
        .ok()
        .map(|key| key.trim().to_string())
        .filter(|key| !key.is_empty())
}

fn api_key_from_secrets_path(path: Option<&std::path::Path>) -> Result<Option<String>> {
    let Some(path) = path else {
        return Ok(None);
    };
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    Ok(serde_json::from_str::<Secrets>(&content)
        .with_context(|| format!("failed to parse {}", path.display()))?
        .api_key)
}

fn read_api_key() -> Result<Option<String>> {
    if let Some(key) = api_key_from_env() {
        return Ok(Some(key));
    }
    api_key_from_secrets_path(secrets_path().as_deref())
}

/// Parse a dollar amount like "$4.50" or "$1,200.00" from text starting at the given prefix.
fn parse_dollar_after(text: &str, prefix: &str) -> Option<f64> {
    let start = text.find(prefix)? + prefix.len();
    leading_number(text.get(start..)?).map(|(value, _)| value)
}

/// Split a leading `-1,234.5`-style number off `text` (an overdrawn free
/// balance prints as `$-1.20`). Only ASCII is consumed, so the returned rest
/// always starts on a char boundary.
fn leading_number(text: &str) -> Option<(f64, &str)> {
    let end = text
        .char_indices()
        .find(|&(i, c)| !(c.is_ascii_digit() || c == '.' || c == ',' || (i == 0 && c == '-')))
        .map_or(text.len(), |(i, _)| i);
    let value: f64 = text[..end].replace(',', "").parse().ok()?;
    value.is_finite().then_some((value, &text[end..]))
}

/// Parse `<remaining><unit> of <total><unit>` right after `marker`.
fn remaining_of_total(line: &str, marker: &str, unit: &str) -> Option<(f64, f64)> {
    let rest = line.get(line.find(marker)? + marker.len()..)?;
    let (remaining, rest) = leading_number(rest)?;
    let rest = rest.strip_prefix(unit)?.strip_prefix(" of ")?;
    let rest = rest.strip_prefix(unit.trim()).unwrap_or(rest);
    let (total, _) = leading_number(rest.strip_prefix('$').unwrap_or(rest))?;
    Some((remaining, total))
}

fn quota_metric(
    label: &str,
    remaining: f64,
    total: f64,
    remaining_label: String,
    resets_at: Option<String>,
) -> Option<UsageMetric> {
    if total <= 0.0 {
        return None;
    }
    let remaining_percent = (remaining / total * 100.0).clamp(0.0, 100.0);
    Some(UsageMetric {
        label: label.into(),
        used_percent: 100.0 - remaining_percent,
        remaining_percent,
        remaining_label: Some(remaining_label),
        resets_at,
    })
}

// Remaining labels share an 11-cell column in the light card and the TUI,
// so drop cents and fractional hours where they do not carry information.
fn format_dollars(amount: f64) -> String {
    if amount.fract() == 0.0 {
        format!("${amount:.0}")
    } else {
        format!("${amount:.2}")
    }
}

fn format_hours(hours: f64) -> String {
    if hours >= 100.0 || hours.fract() == 0.0 {
        format!("{hours:.0}h")
    } else {
        format!("{hours:.1}h")
    }
}

/// `Amp <Plan> Tier: ...` -> (`<Plan>`, body after the colon).
fn split_tier_line(line: &str) -> Option<(&str, &str)> {
    let (head, body) = line.split_once(" Tier:")?;
    Some((head.strip_prefix("Amp ")?.trim(), body))
}

/// `... period 2026-09-18 to 2026-10-18 ...` -> start of the end day (UTC).
fn period_end(line: &str) -> Option<String> {
    let rest = line.get(line.find("period ")?..)?;
    let end = rest.get(rest.find(" to ")? + 4..)?.get(..10)?;
    let date = chrono::NaiveDate::parse_from_str(end, "%Y-%m-%d").ok()?;
    Some(date.and_hms_opt(0, 0, 0)?.and_utc().to_rfc3339())
}

/// `Amp Megawatt Tier: agent usage $26.22 of $60 remaining (44%), orb usage
/// 2,224.8h of 2,250h a1.small orb hours remaining (99%) - period ... to ...`
fn parse_tier_line(body: &str) -> Vec<UsageMetric> {
    let resets_at = period_end(body);
    let agent = remaining_of_total(body, "agent usage $", "").and_then(|(left, total)| {
        quota_metric(
            "Agent",
            left,
            total,
            format!("{}/{}", format_dollars(left), format_dollars(total)),
            resets_at.clone(),
        )
    });
    let orb = remaining_of_total(body, "orb usage ", "h").and_then(|(left, total)| {
        quota_metric(
            "Orb",
            left,
            total,
            format!(
                "{}/{}",
                format_hours(left).trim_end_matches('h'),
                format_hours(total)
            ),
            resets_at.clone(),
        )
    });
    agent.into_iter().chain(orb).collect()
}

/// `Amp Free: $4.71/$10 remaining (replenishes +$0.42/hour)`
fn parse_free_dollars(line: &str) -> Option<UsageMetric> {
    let slash = line.find("/$")?;
    let dollar = line.get(..slash)?.rfind('$')?;
    let (remaining, _) = leading_number(line.get(dollar + 1..slash)?)?;
    let (total, _) = leading_number(line.get(slash + 2..)?)?;
    let used = (total - remaining).max(0.0);
    // Estimate when the allowance is full again from the hourly replenish rate.
    let resets_at = parse_dollar_after(line, "+$")
        .filter(|rate| *rate > 0.0 && used > 0.0)
        .map(|rate| {
            let secs = (used / rate * 3600.0) as i64;
            (chrono::Utc::now() + chrono::Duration::seconds(secs)).to_rfc3339()
        });
    quota_metric(
        "Free",
        remaining,
        total,
        format!("{}/{}", format_dollars(remaining), format_dollars(total)),
        resets_at,
    )
}

/// `Amp Free: 61% remaining today (resets daily)`
fn parse_free_percent(line: &str) -> Option<UsageMetric> {
    let (percent, rest) = leading_number(line.strip_prefix("Amp Free:")?.trim_start())?;
    rest.strip_prefix("% remaining")?;
    quota_metric("Free", percent, 100.0, format!("{percent:.0}% left"), None)
}

/// `Individual credits: $17.23 remaining` or `<Workspace>: $5.33 remaining`
fn parse_credit_line(line: &str) -> Option<UsageMetric> {
    let (name, rest) = line.split_once(": $")?;
    let (credits, rest) = leading_number(rest)?;
    rest.trim_start().strip_prefix("remaining")?;
    let label = if name == "Individual credits" {
        "Credits".to_string()
    } else {
        format!("Credits ({name})")
    };
    Some(UsageMetric {
        label,
        used_percent: 0.0,
        remaining_percent: 100.0,
        remaining_label: Some(format!("${credits:.2} left")),
        resets_at: None,
    })
}

fn parse_display_text(text: &str) -> Vec<UsageMetric> {
    // `amp usage` wraps labels in markdown bold; the API text does not.
    let text = text.replace("**", "");
    let mut metrics = Vec::new();
    for line in text.lines().map(str::trim) {
        if let Some((_, body)) = split_tier_line(line) {
            metrics.extend(parse_tier_line(body));
        } else if let Some(metric) = parse_free_dollars(line)
            .or_else(|| parse_free_percent(line))
            .or_else(|| parse_credit_line(line))
        {
            metrics.push(metric);
        }
    }
    metrics
}

fn detect_plan(text: &str, metrics: &[UsageMetric]) -> Option<String> {
    let text = text.replace("**", "");
    if let Some((plan, _)) = text.lines().find_map(|line| split_tier_line(line.trim())) {
        return Some(plan.to_string());
    }
    let has_free = metrics.iter().any(|m| m.label == "Free");
    let has_credits = metrics.iter().any(|m| m.label.starts_with("Credits"));
    match (has_free, has_credits) {
        (true, _) => Some("Free".into()),
        (false, true) => Some("Credits".into()),
        _ => None,
    }
}

/// `Signed in as user@example.com (org)` -> `user@example.com`
fn parse_email(text: &str) -> Option<String> {
    let rest = text
        .lines()
        .find_map(|line| line.trim().strip_prefix("Signed in as "))?;
    let email = rest.split(" (").next()?.trim();
    (!email.is_empty()).then(|| email.to_string())
}

fn usage_output_from_text(display_text: &str) -> Result<UsageOutput> {
    let metrics = parse_display_text(display_text);
    if metrics.is_empty() {
        anyhow::bail!("Amp returned no parseable usage (display text format may have changed)");
    }
    Ok(UsageOutput {
        freshness: None,
        provider: "Amp".into(),
        account: None,
        credential_source: None,
        plan: detect_plan(display_text, &metrics),
        email: parse_email(display_text),
        metrics,
        reset_credits: None,
        credit_status: None,
        spend_control: None,
    })
}

pub fn has_credentials() -> bool {
    api_key_from_env().is_some() || secrets_path().is_some_and(|path| path.exists())
}

fn fetch_via_cli() -> Option<Result<UsageOutput>> {
    let binary = crate::amp::find_amp_binary()?;
    Some(
        crate::amp::run_amp(&binary, &["usage"], AMP_USAGE_CLI_TIMEOUT)
            .and_then(|stdout| usage_output_from_text(&String::from_utf8_lossy(&stdout))),
    )
}

fn fetch_via_api(api_key: &str) -> Result<UsageOutput> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let display_text = rt.block_on(async {
        let client = super::helpers::http_client()?;
        let resp = client
            .post("https://ampcode.com/api/internal")
            .header("Authorization", format!("Bearer {api_key}"))
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({
                "method": "userDisplayBalanceInfo",
                "params": {}
            }))
            .send()
            .await?;

        if !resp.status().is_success() {
            anyhow::bail!("Amp usage request failed (HTTP {})", resp.status());
        }

        let body: ApiResponse = resp.json().await?;
        if body.ok == Some(false) {
            let error = body.error.as_ref();
            if error.and_then(|e| e.code.as_deref()) == Some("auth-required") {
                anyhow::bail!("Amp API key was rejected. Run `amp login` or update AMP_API_KEY.");
            }
            let msg = error
                .and_then(|e| e.message.as_deref())
                .unwrap_or("unknown error");
            anyhow::bail!("Amp API returned an error: {msg}");
        }
        Ok(body.result.and_then(|r| r.display_text).unwrap_or_default())
    })?;
    usage_output_from_text(&display_text)
}

pub fn fetch() -> Result<UsageOutput> {
    let cli_error = match fetch_via_cli() {
        Some(Ok(output)) => return Ok(output),
        Some(Err(error)) => Some(error),
        None => None,
    };
    match (read_api_key()?, cli_error) {
        (Some(api_key), None) => fetch_via_api(&api_key),
        (Some(api_key), Some(cli_error)) => fetch_via_api(&api_key).map_err(|api_error| {
            anyhow::anyhow!("{api_error} (amp usage also failed: {cli_error:#})")
        }),
        (None, Some(cli_error)) => Err(cli_error),
        (None, None) => anyhow::bail!(
            "No Amp credentials found. Install the amp CLI and run `amp login`, or set AMP_API_KEY."
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // `amp usage` output from a real account (email redacted). The API's
    // displayText is the same text without the markdown bold.
    const TIER_CLI_OUTPUT: &str = "Signed in as user@example.com (overstarry)
**Amp Megawatt Tier:** agent usage $26.22 of $60 remaining (44%), orb usage 2,224.8h of 2,250h a1.small orb hours remaining (99%) - period 2026-09-18 to 2026-10-18, resets upon renewal in 4 days
**Individual credits:** $55.41 remaining (set up auto-reload to avoid running out) - https://ampcode.com/settings

# Run `amp usage --details` for more detailed information.
";

    #[test]
    fn parses_tier_cli_output_into_agent_orb_and_credits() {
        let output = usage_output_from_text(TIER_CLI_OUTPUT).unwrap();
        let labels: Vec<_> = output.metrics.iter().map(|m| m.label.as_str()).collect();
        assert_eq!(labels, vec!["Agent", "Orb", "Credits"]);

        // Percent comes from the dollar figures, not the rounded "(44%)".
        let agent = &output.metrics[0];
        assert!((agent.remaining_percent - 43.7).abs() < 1e-9);
        assert_eq!(agent.remaining_label.as_deref(), Some("$26.22/$60"));
        assert_eq!(
            agent.resets_at.as_deref(),
            Some("2026-10-18T00:00:00+00:00")
        );

        let orb = &output.metrics[1];
        assert_eq!(orb.remaining_label.as_deref(), Some("2225/2250h"));
        assert!((orb.remaining_percent - 2224.8 / 2250.0 * 100.0).abs() < 1e-9);

        assert_eq!(
            output.metrics[2].remaining_label.as_deref(),
            Some("$55.41 left")
        );
        assert_eq!(output.plan.as_deref(), Some("Megawatt"));
        assert_eq!(output.email.as_deref(), Some("user@example.com"));
    }

    #[test]
    fn api_and_cli_text_parse_identically() {
        let api_text = TIER_CLI_OUTPUT.replace("**", "");
        let from_api = usage_output_from_text(&api_text).unwrap();
        let from_cli = usage_output_from_text(TIER_CLI_OUTPUT).unwrap();
        assert_eq!(
            serde_json::to_value(&from_api.metrics).unwrap(),
            serde_json::to_value(&from_cli.metrics).unwrap()
        );
    }

    #[test]
    fn api_response_reads_camel_case_display_text() {
        let body: ApiResponse = serde_json::from_str(
            r#"{"ok":true,"result":{"displayText":"Individual credits: $1.00 remaining"}}"#,
        )
        .unwrap();
        assert_eq!(
            body.result.and_then(|r| r.display_text).as_deref(),
            Some("Individual credits: $1.00 remaining")
        );
    }

    #[test]
    fn parses_free_tier_and_workspace_formats() {
        let text = "Signed in as user@example.com (org)
Amp Free: $4.71/$10 remaining (replenishes +$0.42/hour)
Individual credits: $17.23 remaining
Acme Workspace: $5.33 remaining";
        let output = usage_output_from_text(text).unwrap();
        let labels: Vec<_> = output.metrics.iter().map(|m| m.label.as_str()).collect();
        assert_eq!(labels, vec!["Free", "Credits", "Credits (Acme Workspace)"]);
        assert!((output.metrics[0].remaining_percent - 47.1).abs() < 1e-9);
        assert!(output.metrics[0].resets_at.is_some());
        assert_eq!(output.plan.as_deref(), Some("Free"));

        let daily = parse_display_text("Amp Free: 61% remaining today (resets daily)");
        assert_eq!(daily.len(), 1);
        assert!((daily[0].remaining_percent - 61.0).abs() < 1e-9);
        assert_eq!(daily[0].remaining_label.as_deref(), Some("61% left"));
    }

    #[test]
    fn parses_free_tier_balance() {
        let metrics = parse_display_text("$4.50/$20.00 remaining");
        assert_eq!(metrics.len(), 1);
        assert_eq!(metrics[0].label, "Free");
        // $4.50 remaining of $20.00 -> 77.5% used, 22.5% left.
        assert!((metrics[0].used_percent - 77.5).abs() < 1e-9);
        assert!((metrics[0].remaining_percent - 22.5).abs() < 1e-9);
    }

    #[test]
    fn overdrawn_free_tier_balance_reads_as_fully_used() {
        let metrics = parse_display_text("Amp Free: $-1.20/$10 remaining");
        assert_eq!(metrics.len(), 1);
        assert_eq!(metrics[0].remaining_percent, 0.0);
        assert_eq!(metrics[0].remaining_label.as_deref(), Some("$-1.20/$10"));
    }

    #[test]
    fn empty_display_text_yields_no_metrics() {
        // fetch() bails on an empty metrics set so fetch_all drops the provider
        // instead of rendering a bare header row.
        assert!(parse_display_text("").is_empty());
        assert!(parse_display_text("no dollar figures here").is_empty());
    }

    #[test]
    fn multibyte_display_text_does_not_panic() {
        // Byte offsets from find/rfind must stay on char boundaries; arbitrary
        // UTF-8 around the markers must never panic.
        let _ = parse_display_text("残高 $4.50/$20.00 残り 한국어");
        let _ = parse_display_text("Individual credits: $5.00 残り");
        let _ = parse_dollar_after("プレフィックス€$1.23é", "€$");
        let _ = parse_dollar_after("+$0.50円/hr", "+$");
        // Marker present but followed immediately by a multibyte char.
        let _ = parse_dollar_after("/$é", "/$");
    }

    #[test]
    fn malformed_secrets_file_returns_its_parse_error() {
        let temp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(temp.path(), "not json").unwrap();
        let error = api_key_from_secrets_path(Some(temp.path())).unwrap_err();
        assert!(error.to_string().contains("failed to parse"));
    }
}
