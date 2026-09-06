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

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
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
    off_caller_runtime(|| {
        let Ok(rt) = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        else {
            return false;
        };
        rt.block_on(async { discover_port().await.is_some() })
    })
    .unwrap_or(false)
}

pub fn fetch_all() -> Result<Vec<UsageOutput>> {
    off_caller_runtime(|| {
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
    })
    .unwrap_or_else(|_| Err(anyhow::anyhow!("Antigravity usage worker thread panicked")))
}

/// Run `work` on a dedicated OS thread, off whatever runtime the caller is on.
///
/// Both entry points above drive their own current-thread runtime, and
/// `block_on` panics with "Cannot start a runtime from within a runtime" when
/// the calling thread already has a Tokio runtime context entered (#1264). No
/// caller does that today -- `tokscale usage` reaches them from a plain sync
/// `main`, and the TUI usage refresh runs the fetcher on a bare
/// `std::thread::spawn` -- so this is hardening rather than a live crash path:
/// a fresh OS thread never carries a runtime context, which keeps the panic
/// impossible for whatever entry point is added next. The scope joins before
/// returning, so both entry points stay as synchronous as they were.
/// `crate::antigravity::https_rpc_request` isolates its own `block_on` the
/// same way, and that one is reachable today: `discover_port` walks into it
/// synchronously from inside the runtime `has_credentials` just entered.
fn off_caller_runtime<T: Send>(work: impl FnOnce() -> T + Send) -> std::thread::Result<T> {
    std::thread::scope(|scope| scope.spawn(work).join())
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
        metrics: group.buckets.into_iter().filter_map(metric).collect(),
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

/// Build a metric for one bucket, or `None` when the server did not say how
/// much is left.
///
/// Defaulting a missing `remainingFraction` to zero renders as "100% used" --
/// a full-exhaustion warning invented out of an absent field. Version skew or a
/// partial response is exactly when that would fire, so a bucket that cannot
/// state its own remainder is dropped instead of being reported as spent. A
/// group whose buckets all drop shows no rows rather than a false alarm.
fn metric(bucket: QuotaBucket) -> Option<UsageMetric> {
    // The wire format reports what is **left**; `UsageMetric` leads with what
    // has been used. Getting this backwards turns "7% left" into "7% used",
    // which is the most dangerous way to be wrong about a quota.
    let remaining = bucket
        .remaining_fraction
        .filter(|fraction| fraction.is_finite())?
        .clamp(0.0, 1.0)
        * 100.0;

    Some(UsageMetric {
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
    })
}

/// Byte ceiling for one quota response.
///
/// `PROBE_TIMEOUT` bounds how long the language server may take, not how much
/// it may send inside that window, and `Response::json` buffers the whole body
/// before anything looks at it. Port discovery probes candidate ports, so this
/// also runs against whatever else happens to be listening on loopback -- a
/// port that answers with an endless stream would otherwise allocate until the
/// timeout, and usage providers share a fan-out, so that takes the whole
/// `tokscale usage` report down rather than one provider.
///
/// A real summary is a handful of bucket objects well under a kilobyte; 1 MiB
/// leaves room for new fields while keeping the worst case bounded.
const MAX_QUOTA_BODY_BYTES: usize = 1024 * 1024;

async fn call_rpc(port: u16) -> Result<QuotaSummary> {
    // `.no_proxy()` because this only ever targets 127.0.0.1: the default
    // builder honours HTTP_PROXY/system proxy settings, which would send a
    // loopback quota request to a remote host unless the user happens to have
    // a matching NO_PROXY. That both leaks quota metadata and lets the proxy
    // forge the unauthenticated response that port discovery trusts. The IDE
    // RPC client in `crate::antigravity` is built the same way.
    let client = tokscale_core::http::client_builder()
        .no_proxy()
        // Redirects are refused for the same reason the proxy is: discovery
        // probes ports that may belong to anything, and reqwest follows up to
        // ten redirects by default. A stale or hostile local listener answering
        // 307 with an external URL would carry the request off loopback, and
        // the remote answer would then be accepted as a quota summary.
        .redirect(reqwest::redirect::Policy::none())
        .timeout(PROBE_TIMEOUT)
        .build()?;
    let response = client
        .post(format!("http://127.0.0.1:{port}{RPC_PATH}"))
        // Connect-RPC rejects the request without this header.
        .header("Connect-Protocol-Version", "1")
        .json(&serde_json::json!({}))
        .send()
        .await?
        .error_for_status()?;
    let body =
        crate::antigravity::read_reqwest_response_with_cap(response, MAX_QUOTA_BODY_BYTES).await?;
    let envelope: QuotaSummaryEnvelope = serde_json::from_str(&body)?;
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

/// Bytes of `cli.log` read when looking for logged ports.
///
/// The log is appended across every CLI run and nothing rotates it, so reading
/// it whole grows without bound on a long-lived install. Only the most recent
/// entries matter here -- the tail is where the current port is -- so reading
/// the end is both bounded and the answer this function actually wants.
const CLI_LOG_TAIL_BYTES: u64 = 256 * 1024;

/// Read the last `max_bytes` of a file, or the whole file when it is smaller.
///
/// The leading partial line the offset can land in is dropped by the caller's
/// line parse, which requires a whole `listening on random port at NNNN` match.
fn read_tail(path: &Path, max_bytes: u64) -> Option<String> {
    let mut file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    if len > max_bytes {
        file.seek(SeekFrom::Start(len - max_bytes)).ok()?;
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).ok()?;
    // Lossy because the tail offset can split a multi-byte character, and a
    // replacement char in a line this parse ignores costs nothing.
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// Ports the CLI logged, most recent first.
fn ports_from_cli_log() -> Vec<u16> {
    let Some(path) = cli_log_path() else {
        return Vec::new();
    };
    let Some(text) = read_tail(&path, CLI_LOG_TAIL_BYTES) else {
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

    /// The log is appended across every run and never rotated, so the read has
    /// to be bounded -- and the bound has to keep the *end*, because that is
    /// where the port of the currently running server was written.
    #[test]
    fn reads_the_end_of_an_oversized_log() {
        use std::io::Write;

        let mut file = tempfile::NamedTempFile::new().unwrap();
        let filler = "noise from an earlier run\n".repeat(4096);
        write!(file, "{filler}").unwrap();
        writeln!(file, "listening on random port at 41234 for HTTP").unwrap();
        file.flush().unwrap();

        let tail = read_tail(file.path(), 512).expect("the log is readable");
        assert!(
            tail.len() as u64 <= 512,
            "read {} bytes past the 512 byte ceiling",
            tail.len()
        );
        assert_eq!(
            parse_logged_ports(&tail),
            vec![41234],
            "the most recent port must survive the truncation"
        );
    }

    #[test]
    fn reads_a_whole_log_smaller_than_the_ceiling() {
        use std::io::Write;

        let mut file = tempfile::NamedTempFile::new().unwrap();
        writeln!(file, "listening on random port at 5001 for HTTP").unwrap();
        writeln!(file, "listening on random port at 5002 for HTTP").unwrap();
        file.flush().unwrap();

        let tail = read_tail(file.path(), CLI_LOG_TAIL_BYTES).expect("the log is readable");
        assert_eq!(parse_logged_ports(&tail), vec![5001, 5002]);
    }

    #[test]
    fn remaining_fraction_becomes_used_percent() {
        let m = metric(QuotaBucket {
            display_name: "Weekly Limit Remaining".to_string(),
            window: Some("weekly".to_string()),
            remaining_fraction: Some(0.414_529_86),
            reset_time: Some("2026-08-29T03:58:44Z".to_string()),
        })
        .expect("a bucket that states its remainder yields a metric");

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
        })
        .expect("a bucket that states its remainder yields a metric");
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
            })
            .expect("a finite fraction yields a metric");
            assert!((0.0..=100.0).contains(&m.remaining_percent));
            assert!((0.0..=100.0).contains(&m.used_percent));
        }
    }

    /// A bucket that does not say how much is left must not be rendered as
    /// fully spent. Defaulting the absent value to zero remaining shows "100%
    /// used" -- an exhaustion warning invented from a missing field, and
    /// version skew or a truncated response is exactly when it would fire.
    #[test]
    fn a_bucket_without_a_usable_remainder_is_dropped_not_reported_as_spent() {
        for fraction in [
            None,
            Some(f64::NAN),
            Some(f64::INFINITY),
            Some(f64::NEG_INFINITY),
        ] {
            let dropped = metric(QuotaBucket {
                display_name: "Weekly Limit Remaining".to_string(),
                window: Some("weekly".to_string()),
                remaining_fraction: fraction,
                reset_time: None,
            });
            assert!(
                dropped.is_none(),
                "remaining_fraction={fraction:?} must not render as a quota row"
            );
        }
    }

    /// The drop is per bucket: a group keeps the buckets that are readable.
    #[test]
    fn a_group_keeps_its_readable_buckets_when_one_is_unusable() {
        let output = output_for_group(QuotaGroup {
            display_name: "Gemini Models".to_string(),
            buckets: vec![
                QuotaBucket {
                    display_name: "Weekly".to_string(),
                    window: Some("weekly".to_string()),
                    remaining_fraction: None,
                    reset_time: None,
                },
                QuotaBucket {
                    display_name: "Five Hour".to_string(),
                    window: Some("5h".to_string()),
                    remaining_fraction: Some(0.25),
                    reset_time: None,
                },
            ],
        });

        assert_eq!(output.metrics.len(), 1);
        assert_eq!(output.metrics[0].label, "5h");
        assert!((output.metrics[0].used_percent - 75.0).abs() < 1e-6);
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

    /// Regression for #1264. Both entry points build their own current-thread
    /// runtime, and `block_on` panics with "Cannot start a runtime from within
    /// a runtime" when the calling thread already has a runtime context
    /// entered. No caller does that today, so what is pinned here is hardening.
    ///
    /// Calling them is not sufficient on its own. Both swallow a worker panic
    /// (`unwrap_or(false)`, `unwrap_or_else`), so a worker that still panicked
    /// inside the caller's runtime returns exactly like a working one, and
    /// neither return value can be asserted on because whether a language
    /// server answers is a property of the machine. Hence the assert, which
    /// pins the isolation itself: work left on the caller's thread sees the
    /// entered runtime context, and only work moved off that thread does not.
    /// The two calls still carry the other half -- an entry point that stops
    /// using the helper panics on them while the assert stays green -- so keep
    /// both halves.
    #[test]
    fn entry_points_tolerate_an_entered_tokio_runtime_context() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        rt.block_on(async {
            assert!(
                off_caller_runtime(|| tokio::runtime::Handle::try_current().is_err())
                    .expect("the worker must not panic"),
                "the work must run off the caller's runtime"
            );
            let _ = has_credentials();
            let _ = fetch_all();
        });
    }

    #[test]
    fn group_slugs_are_stable_and_url_safe() {
        assert_eq!(slug("Gemini Models"), "gemini-models");
        assert_eq!(slug("Claude and GPT models"), "claude-and-gpt-models");
        assert_eq!(slug("  spaced  "), "spaced");
    }
}
