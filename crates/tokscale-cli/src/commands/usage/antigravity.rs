//! Antigravity subscription quota.
//!
//! Unlike every other provider here, this one never reaches a cloud API. The
//! Antigravity CLI (`agy`) and IDE both run a language server that holds the
//! OAuth token, calls Google Code Assist, caches the answer, and exposes it
//! over a loopback Connect-RPC endpoint. `/usage` inside the CLI reads exactly
//! that:
//!
//! ```text
//! POST http://127.0.0.1:<port>/exa.language_server_pb.LanguageServerService/RetrieveUserQuotaSummary
//! Connect-Protocol-Version: 1
//! {}
//! ```
//!
//! No CSRF token is required for this method, and tokscale never has to hold
//! an Antigravity credential of its own — the token stays inside the language
//! server and we only read the numbers it already computed.
//!
//! Calling Google's `cloudcode-pa.googleapis.com/v1internal:retrieveUserQuota`
//! directly was tried first and rejected: authentication succeeds, but a
//! `GEMINI_CLI` client identity is answered with `UNSUPPORTED_CLIENT` /
//! `SUBSCRIPTION_REQUIRED` now that individual users have been migrated to
//! Antigravity.
//!
//! Antigravity meters quota **per model group** — Gemini models share one
//! weekly and one five-hour limit, Claude and GPT models share another — so
//! this provider emits one [`UsageOutput`] per group and names the group in
//! the account label. That reuses the existing multi-output path (the same one
//! Codex uses for multiple accounts) and renders as
//! `Antigravity (Gemini Models)`, rather than flattening every bucket into one
//! list where no row can be attributed to a group.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use serde::Deserialize;

use super::{UsageAccount, UsageMetric, UsageOutput};

const PROVIDER: &str = "Antigravity";
const RPC_PATH: &str = "/exa.language_server_pb.LanguageServerService/RetrieveUserQuotaSummary";

/// The loopback call itself answers in a few milliseconds. This bound only
/// matters when a candidate port belongs to some unrelated process that
/// accepts the connection and then goes quiet.
const PROBE_TIMEOUT: Duration = Duration::from_millis(400);

// ── Wire format ──

#[derive(Debug, Deserialize)]
struct QuotaSummaryEnvelope {
    response: QuotaSummary,
}

#[derive(Debug, Deserialize)]
struct QuotaSummary {
    #[serde(default)]
    groups: Vec<QuotaGroup>,
}

#[derive(Debug, Deserialize)]
struct QuotaGroup {
    #[serde(rename = "displayName", default)]
    display_name: String,
    #[serde(default)]
    buckets: Vec<QuotaBucket>,
}

#[derive(Debug, Deserialize)]
struct QuotaBucket {
    #[serde(rename = "displayName", default)]
    display_name: String,
    /// `"weekly"` or `"5h"`.
    #[serde(default)]
    window: Option<String>,
    /// Fraction **remaining**, 0.0 to 1.0 — not the fraction used.
    #[serde(rename = "remainingFraction", default)]
    remaining_fraction: Option<f64>,
    #[serde(rename = "resetTime", default)]
    reset_time: Option<String>,
}

// ── Provider interface ──

/// Whether a reachable language server is serving quota right now.
///
/// This probes rather than checking for a credential file, because Antigravity
/// keeps its token inside the running process: "is it installed" and "can we
/// read quota" are different questions, and only the second one should put a
/// card on screen. The probe is a loopback request against a port read out of
/// the CLI log, so it costs a few milliseconds.
pub fn has_credentials() -> bool {
    let Ok(rt) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        return false;
    };
    rt.block_on(async { discover_port().await.is_some() })
}

pub fn fetch_all() -> Result<Vec<UsageOutput>> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    rt.block_on(async {
        let port = discover_port()
            .await
            .context("Antigravity language server is not running")?;
        let summary = call_rpc(port).await?;

        if summary.groups.is_empty() {
            anyhow::bail!("Antigravity is running but not signed in");
        }

        Ok(summary.groups.into_iter().map(output_for_group).collect())
    })
}

fn output_for_group(group: QuotaGroup) -> UsageOutput {
    let name = group.display_name;
    UsageOutput {
        provider: PROVIDER.to_string(),
        // The "account" slot carries the model group. Antigravity has a single
        // account but several independently metered groups, and this is the
        // field the renderer already appends in parentheses.
        account: Some(UsageAccount {
            id: slug(&name),
            label: Some(name),
            is_active: true,
        }),
        credential_source: None,
        plan: None,
        email: None,
        metrics: group.buckets.into_iter().map(metric).collect(),
        reset_credits: None,
        credit_status: None,
        spend_control: None,
    }
}

/// Stable identifier for a group, derived from its display name.
fn slug(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

fn metric(bucket: QuotaBucket) -> UsageMetric {
    // The wire format reports what is **left**; `UsageMetric` leads with what
    // has been used. Getting this backwards turns "7% left" into "7% used",
    // which is the most dangerous way to be wrong about a quota.
    let remaining = bucket.remaining_fraction.unwrap_or(0.0).clamp(0.0, 1.0) * 100.0;

    UsageMetric {
        // `displayName` reads "Weekly Limit Remaining", which is too long for a
        // card once the group name is also shown; `window` is the short form.
        label: bucket
            .window
            .filter(|w| !w.is_empty())
            .unwrap_or(bucket.display_name),
        used_percent: 100.0 - remaining,
        remaining_percent: remaining,
        remaining_label: None,
        resets_at: bucket.reset_time,
    }
}

async fn call_rpc(port: u16) -> Result<QuotaSummary> {
    let client = reqwest::Client::builder().timeout(PROBE_TIMEOUT).build()?;
    let envelope: QuotaSummaryEnvelope = client
        .post(format!("http://127.0.0.1:{port}{RPC_PATH}"))
        // Connect-RPC rejects the request without this header.
        .header("Connect-Protocol-Version", "1")
        .json(&serde_json::json!({}))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    Ok(envelope.response)
}

// ── Port discovery ──

/// Find a language server port that answers `RetrieveUserQuotaSummary`.
///
/// Two sources, cheapest first:
///
/// 1. The CLI log, which records `listening on random port at NNNN for HTTP`
///    on every start. Reading one file beats enumerating processes.
/// 2. [`crate::antigravity::detect_antigravity_connections`], which finds the
///    IDE's language server. That path needs a CSRF token on the process
///    command line, which the `agy` CLI does not have — hence source 1.
///
/// Candidates are probed rather than trusted: the process listens on both an
/// HTTPS (gRPC) port and an HTTP one, and only the latter speaks plain JSON.
async fn discover_port() -> Option<u16> {
    for port in ports_from_cli_log() {
        if call_rpc(port).await.is_ok() {
            return Some(port);
        }
    }

    for connection in crate::antigravity::detect_antigravity_connections().ok()? {
        if call_rpc(connection.port).await.is_ok() {
            return Some(connection.port);
        }
    }

    None
}

fn cli_log_path() -> Option<PathBuf> {
    Some(
        tokscale_core::paths::home_dir()?
            .join(".gemini")
            .join("antigravity-cli")
            .join("cli.log"),
    )
}

/// Ports the CLI logged, most recent first.
fn ports_from_cli_log() -> Vec<u16> {
    let Some(path) = cli_log_path() else {
        return Vec::new();
    };
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut ports = parse_logged_ports(&text);
    // The log is appended across runs, so the last entry is the current one.
    ports.reverse();
    ports.truncate(4);
    ports
}

fn parse_logged_ports(text: &str) -> Vec<u16> {
    const MARKER: &str = "listening on random port at ";
    text.lines()
        .filter(|line| line.contains("for HTTP") && !line.contains("for HTTPS"))
        .filter_map(|line| {
            let rest = &line[line.find(MARKER)? + MARKER.len()..];
            let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
            digits.parse::<u16>().ok().filter(|p| *p != 0)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remaining_fraction_becomes_used_percent() {
        let m = metric(QuotaBucket {
            display_name: "Weekly Limit Remaining".to_string(),
            window: Some("weekly".to_string()),
            remaining_fraction: Some(0.414_529_86),
            reset_time: Some("2026-08-29T03:58:44Z".to_string()),
        });

        assert_eq!(m.label, "weekly");
        assert!((m.remaining_percent - 41.452_986).abs() < 1e-6);
        assert!((m.used_percent - 58.547_014).abs() < 1e-6);
    }

    #[test]
    fn bucket_without_a_window_falls_back_to_its_display_name() {
        let m = metric(QuotaBucket {
            display_name: "Weekly Limit Remaining".to_string(),
            window: None,
            remaining_fraction: Some(1.0),
            reset_time: None,
        });
        assert_eq!(m.label, "Weekly Limit Remaining");
    }

    #[test]
    fn out_of_range_fractions_are_clamped() {
        for fraction in [-1.0, 0.0, 1.0, 4.2] {
            let m = metric(QuotaBucket {
                display_name: "x".to_string(),
                window: None,
                remaining_fraction: Some(fraction),
                reset_time: None,
            });
            assert!((0.0..=100.0).contains(&m.remaining_percent));
            assert!((0.0..=100.0).contains(&m.used_percent));
        }
    }

    #[test]
    fn log_parsing_takes_the_http_port_and_skips_the_grpc_one() {
        let log = concat!(
            "I0827 15:07:44 server.go:599] Language server listening on random port at 2578 for HTTPS (gRPC)\n",
            "I0827 15:07:44 server.go:607] Language server listening on random port at 2579 for HTTP\n",
        );
        assert_eq!(parse_logged_ports(log), vec![2579]);
    }

    #[test]
    fn log_parsing_skips_malformed_lines() {
        let log = "listening on random port at abc for HTTP\nunrelated line\n";
        assert!(parse_logged_ports(log).is_empty());
    }

    #[test]
    fn quota_summary_parses_a_recorded_response() {
        let raw = r#"{
          "response": {
            "groups": [
              {
                "displayName": "Gemini Models",
                "description": "Models within this group: Gemini Flash, Gemini Pro",
                "buckets": [
                  {
                    "bucketId": "gemini-weekly",
                    "displayName": "Weekly Limit Remaining",
                    "window": "weekly",
                    "remainingFraction": 0.41452986,
                    "resetTime": "2026-08-29T03:58:44Z"
                  },
                  {
                    "bucketId": "gemini-5h",
                    "displayName": "Five Hour Limit Remaining",
                    "window": "5h",
                    "remainingFraction": 1
                  }
                ]
              },
              {
                "displayName": "Claude and GPT models",
                "buckets": [
                  {
                    "bucketId": "3p-weekly",
                    "window": "weekly",
                    "remainingFraction": 0.89853734
                  }
                ]
              }
            ]
          }
        }"#;

        let envelope: QuotaSummaryEnvelope = serde_json::from_str(raw).expect("parses");
        let outputs: Vec<UsageOutput> = envelope
            .response
            .groups
            .into_iter()
            .map(output_for_group)
            .collect();

        assert_eq!(outputs.len(), 2, "one output per model group");

        let gemini = &outputs[0];
        assert_eq!(gemini.provider, "Antigravity");
        assert_eq!(
            gemini.account.as_ref().unwrap().label.as_deref(),
            Some("Gemini Models")
        );
        assert_eq!(gemini.metrics.len(), 2);
        assert!((gemini.metrics[0].remaining_percent - 41.452_986).abs() < 1e-6);
        assert!((gemini.metrics[1].remaining_percent - 100.0).abs() < 1e-9);

        let third_party = &outputs[1];
        assert_eq!(
            third_party.account.as_ref().unwrap().label.as_deref(),
            Some("Claude and GPT models")
        );
        assert_eq!(
            third_party.account.as_ref().unwrap().id,
            "claude-and-gpt-models"
        );
    }

    #[test]
    fn group_slugs_are_stable_and_url_safe() {
        assert_eq!(slug("Gemini Models"), "gemini-models");
        assert_eq!(slug("Claude and GPT models"), "claude-and-gpt-models");
        assert_eq!(slug("  spaced  "), "spaced");
    }
}
