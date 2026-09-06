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

/// Deadline for one round of candidate requests.
///
/// What this replaces was 400ms *per candidate*, applied to a full quota
/// request. It read as a probe bound, but it was also the only gate on
/// availability: [`has_credentials`] reports a language server as present
/// exactly when one answers a quota summary, and `usage/mod.rs` drops a
/// provider from the report before any fetch runs when it does not. A language
/// server on a loaded machine pushes a loopback round trip well past 400ms, so
/// the bound written to abandon *wrong* ports was abandoning the right one
/// (#1280).
///
/// Asking the candidates together is what makes the longer bound affordable. A
/// port nothing is listening on refuses the connection outright, so a machine
/// without Antigravity never approaches this, and a port that accepts and then
/// goes quiet is waited on once for the round rather than once per candidate.
///
/// A *round*, not the whole of discovery. [`discover_quota`] has two candidate
/// sources and each gets its own round, computed when that round starts, so
/// the requests can cost twice this in total. One instant shared across both
/// was worse than that arithmetic: a first source that spent it handed the
/// second an expired deadline, and `tokio::time::timeout_at` resolves an
/// expired deadline by polling once and dropping the task set, so the second
/// source's requests were abandoned before a socket was opened. Neither round
/// covers [`crate::antigravity::detect_antigravity_connections`], which is
/// synchronous and carries per-socket timeouts of its own.
///
/// Bounded at all because usage providers share a fan-out: whatever this
/// spends is spent by the whole `tokscale usage` report. 10s across both
/// rounds sits well inside the 12s-30s the cloud-backed providers here allow
/// themselves, and leaves a healthy loopback answer -- milliseconds when idle,
/// low seconds on a loaded machine -- several times the room it needs.
///
/// This is also how long a round waits for a *better-ranked* candidate once it
/// holds an answer from a worse one. [`ports_from_cli_log`] yields candidates
/// newest first, because the last port the log names is the current server,
/// and an older entry can still be answered by a stale `agy` left over from a
/// restart or an account switch, whose quota is another account's or nobody's
/// (signed out, which parses fine and reports as "not signed in"). So a round
/// keeps the best-ranked answer it has seen and returns the moment nothing
/// better can still arrive. What it is waiting on until then is the current
/// server -- the server this budget was sized for -- so there is no shorter
/// grace for that wait. An earlier revision cut it at 2s from the first answer,
/// which contradicted the premise above: a current server answering in 2-5s on
/// a loaded machine lost to a stale sibling that answered at once. The cost of
/// a single bound is confined to a newest logged port that accepts and then
/// goes quiet while an older one still answers: that round runs to its
/// deadline before believing the older answer, which is the worst case one
/// stalled candidate already costs.
const DISCOVERY_ROUND_TIMEOUT: Duration = Duration::from_secs(5);

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
/// the CLI log, so on a machine that has one it costs a few milliseconds. A
/// machine with neither a logged port nor an Antigravity process sends no
/// request at all and builds no HTTP client: what it pays is the log read and
/// the process scan.
///
/// This runs the same [`discover_quota`] the fetch runs, so the two cannot
/// disagree about whether a server is answering: a summary that clears this
/// gate is a summary the fetch can also obtain.
pub fn has_credentials() -> bool {
    off_caller_runtime(|| {
        let Ok(rt) = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        else {
            return false;
        };
        rt.block_on(async { discover_quota().await.is_some() })
    })
    .unwrap_or(false)
}

pub fn fetch_all() -> Result<Vec<UsageOutput>> {
    off_caller_runtime(|| {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;

        rt.block_on(async {
            let summary = discover_quota()
                .await
                .context("Antigravity language server is not running")?;

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
/// same way, and that one is reachable today: [`detected_ports`] walks into it
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
/// [`DISCOVERY_ROUND_TIMEOUT`] bounds how long the language server may take,
/// not how much it may send inside that window, and the body is buffered whole
/// before anything looks at it. Discovery asks candidate ports, so this runs
/// against whatever else happens to be listening on loopback -- a port that
/// answers with an endless stream would otherwise allocate until the timeout,
/// and usage providers share a fan-out, so that takes the whole `tokscale
/// usage` report down rather than one provider.
///
/// A real summary is a handful of bucket objects well under a kilobyte; 1 MiB
/// leaves room for new fields while keeping the worst case bounded.
const MAX_QUOTA_BODY_BYTES: usize = 1024 * 1024;

/// The client every candidate request in a round shares.
///
/// One per round rather than one per request, because `ClientBuilder::build`
/// is synchronous and would otherwise serialise the requests the round exists
/// to overlap. `reqwest::Client` is internally reference counted, so handing a
/// clone to each request is free.
///
/// Built only once a round has a candidate to send to -- see
/// [`race_for_quota`], which returns before reaching here on an empty list.
fn quota_client() -> Result<reqwest::Client> {
    // `.no_proxy()` because this only ever targets 127.0.0.1: the default
    // builder honours HTTP_PROXY/system proxy settings, which would send a
    // loopback quota request to a remote host unless the user happens to have
    // a matching NO_PROXY. That both leaks quota metadata and lets the proxy
    // forge the unauthenticated response that port discovery trusts. The IDE
    // RPC client in `crate::antigravity` is built the same way.
    Ok(tokscale_core::http::client_builder()
        .no_proxy()
        // Redirects are refused for the same reason the proxy is: discovery
        // asks ports that may belong to anything, and reqwest follows up to ten
        // redirects by default. A stale or hostile local listener answering 307
        // with an external URL would carry the request off loopback, and the
        // remote answer would then be accepted as a quota summary.
        .redirect(reqwest::redirect::Policy::none())
        // No trust store. Every request this client sends is
        // `http://127.0.0.1:<port>` -- there is no certificate to check, and
        // the redirect that could carry it to https is refused above. Loading
        // the platform roots anyway is neither free nor cached: reqwest
        // 0.12.28 calls `rustls_native_certs::load_native_certs()` inside
        // every `build()`, measured here at 148-221ms per build with the sixth
        // costing as much as the first, against 4-19us with this off. That was
        // latency the whole report paid, because `has_credentials` runs on
        // `usage/mod.rs`'s sequential pre-flight loop rather than in its
        // fan-out.
        .tls_built_in_native_certs(false)
        // The round is what bounds discovery in practice, since it abandons
        // every request still in flight at its deadline. This is the backstop
        // that keeps a `call_rpc` awaited on its own from being unbounded.
        .timeout(DISCOVERY_ROUND_TIMEOUT)
        .build()?)
}

async fn call_rpc(client: reqwest::Client, port: u16) -> Result<QuotaSummary> {
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

/// Find a language server that answers `RetrieveUserQuotaSummary`, and keep
/// what it answered.
///
/// Two sources, cheapest first:
///
/// 1. The CLI log, which records `listening on random port at NNNN for HTTP`
///    on every start. Reading one file beats enumerating processes.
/// 2. [`crate::antigravity::detect_antigravity_connections`], which finds the
///    IDE's language server. That path needs a CSRF token on the process
///    command line, which the `agy` CLI does not have — hence source 1.
///
/// Candidates are asked rather than trusted: the process listens on both an
/// HTTPS (gRPC) port and an HTTP one, and only the latter speaks plain JSON.
/// The question *is* the quota request, so the summary that proves a port good
/// is the summary returned. `fetch_all` used to throw it away and ask the same
/// port the same question again, which is a second round trip for an answer
/// already in hand.
async fn discover_quota() -> Option<QuotaSummary> {
    discover_quota_from(&[&ports_from_cli_log, &detected_ports]).await
}

/// Ask each source in turn and stop at the first that answers.
///
/// Separate from [`discover_quota`] so the sequencing can be tested: source 2
/// runs `ps` and has no fixture, so the only way to observe that a first
/// source which spent its whole round still leaves the second one asked is to
/// hand discovery two sources the test controls.
async fn discover_quota_from(sources: &[&dyn Fn() -> Vec<u16>]) -> Option<QuotaSummary> {
    for source in sources {
        if let Some(summary) = race_for_quota(source()).await {
            return Some(summary);
        }
    }
    None
}

/// Ports of the IDE language servers found by scanning processes.
///
/// Reached only when the log offered nothing, which keeps `ps` plus a
/// heartbeat per port off the common path. Unlike source 1 this is
/// synchronous and brings its own per-socket timeouts, so it sits outside
/// either round's deadline.
fn detected_ports() -> Vec<u16> {
    crate::antigravity::detect_antigravity_connections()
        .map(|connections| {
            connections
                .into_iter()
                .map(|connection| connection.port)
                .collect()
        })
        .unwrap_or_default()
}

/// Ask every candidate at once and keep the best answer.
///
/// Asking in sequence made each candidate wait out the one before it, which is
/// what forced a budget short enough to be paid several times over. Sent
/// together, a candidate that stalls costs the others nothing, so
/// [`DISCOVERY_ROUND_TIMEOUT`] -- computed here, when this round starts, never
/// inherited from a round that already ran -- is the only bound left. Losing
/// requests are cancelled when the set is dropped.
///
/// "Best" is by position rather than by who finishes first: `ports` arrives
/// newest first and the newest is the one the log calls current. The round
/// returns the moment nothing better can still arrive, and until then waits
/// for a better-ranked candidate exactly as long as it waits for any --
/// [`DISCOVERY_ROUND_TIMEOUT`] says why that is one bound and not two.
///
/// An empty `ports` returns before building anything, which is what keeps a
/// machine with no Antigravity from paying for a client it has nothing to
/// send to.
async fn race_for_quota(ports: Vec<u16>) -> Option<QuotaSummary> {
    race_for_quota_with(ports, &quota_client).await
}

/// [`race_for_quota`] with the client build handed in.
///
/// Separate so a test can count the builds one call causes from that call:
/// whether an empty round builds a client has no other observable -- the
/// regression it guards was a client built before discovery knew whether it
/// had anywhere to send, and the only symptom was time. A count kept inside
/// `quota_client` instead is fed by every round in the process, on every
/// thread, and puts a `#[cfg(test)]` hook in shipped code.
async fn race_for_quota_with(
    ports: Vec<u16>,
    build_client: &dyn Fn() -> Result<reqwest::Client>,
) -> Option<QuotaSummary> {
    if ports.is_empty() {
        return None;
    }

    let client = build_client().ok()?;
    let deadline = tokio::time::Instant::now() + DISCOVERY_ROUND_TIMEOUT;
    let mut settled = vec![false; ports.len()];
    let mut requests = tokio::task::JoinSet::new();
    for (rank, port) in ports.into_iter().enumerate() {
        let client = client.clone();
        requests.spawn(async move { (rank, call_rpc(client, port).await.ok()) });
    }

    let mut best: Option<(usize, QuotaSummary)> = None;

    while let Ok(Some(joined)) = tokio::time::timeout_at(deadline, requests.join_next()).await {
        // A `JoinError` carries no rank, so a panicked request cannot be
        // marked settled and the round runs to its deadline instead of
        // returning early. `call_rpc` reports every failure it has as `Err`,
        // so this is the unreachable arm rather than the failure path.
        let Ok((rank, answer)) = joined else { continue };
        settled[rank] = true;
        if let Some(summary) = answer {
            if best.as_ref().is_none_or(|(kept, _)| rank < *kept) {
                best = Some((rank, summary));
            }
        }
        let Some((kept, _)) = best.as_ref() else {
            continue;
        };
        if settled[..*kept].iter().all(|done| *done) {
            break;
        }
    }

    best.map(|(_, summary)| summary)
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
    use crate::commands::usage::test_server::{spawn_server, Seen};
    use crate::commands::usage::{usage_providers, Fetch};
    use serial_test::serial;
    use std::sync::{Arc, Mutex};
    use tempfile::TempDir;

    /// RAII restore of the process-global home redirect, mirroring the copy in
    /// `super::copilot`'s tests (`tokscale_core::paths::test_env::EnvGuard` is
    /// `pub(crate)` to the core crate and cannot be imported here). Restoring
    /// on `Drop` rather than at the end of the body matters because a failing
    /// assertion panics first, and the next test would then resolve `HOME` to
    /// a deleted `TempDir`.
    struct EnvGuard(Vec<(&'static str, Option<std::ffi::OsString>)>);

    impl EnvGuard {
        fn capture(keys: &[&'static str]) -> Self {
            Self(
                keys.iter()
                    .map(|key| (*key, std::env::var_os(key)))
                    .collect(),
            )
        }

        fn set(&mut self, key: &str, value: impl AsRef<std::ffi::OsStr>) {
            unsafe { std::env::set_var(key, value) };
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (key, previous) in self.0.drain(..) {
                unsafe {
                    match previous {
                        Some(value) => std::env::set_var(key, value),
                        None => std::env::remove_var(key),
                    }
                }
            }
        }
    }

    /// `dirs::home_dir()` falls back to `USERPROFILE` whenever the `HOME`
    /// override is rejected, so both have to point at the fixture or the
    /// Windows leg reads the runner's real `cli.log`.
    const HOME_ENV_KEYS: [&str; 2] = ["HOME", "USERPROFILE"];

    /// One quota group, named distinctly enough that a real language server
    /// running on the machine under test cannot be mistaken for the fixture.
    const FIXTURE_QUOTA: &str = r#"{"response":{"groups":[{"displayName":"Fixture Models","buckets":[{"displayName":"Weekly Limit Remaining","window":"weekly","remainingFraction":0.25}]}]}}"#;

    /// Point the home-rooted `cli.log` lookup at a fixture naming `probe_order`.
    ///
    /// Written back to front: the log is appended across runs, so
    /// [`ports_from_cli_log`] treats the *last* entry as the current port and
    /// reverses what it parsed. A call site that wants a port tried last has to
    /// put it first in the file, which is worth hiding here.
    fn redirect_home_to_logged_ports(env: &mut EnvGuard, home: &TempDir, probe_order: &[u16]) {
        let log_dir = home.path().join(".gemini").join("antigravity-cli");
        std::fs::create_dir_all(&log_dir).unwrap();
        let log: String = probe_order
            .iter()
            .rev()
            .map(|port| {
                format!(
                    "I0827 15:07:44 server.go:607] Language server listening on random port at {port} for HTTP\n"
                )
            })
            .collect();
        std::fs::write(log_dir.join("cli.log"), log).unwrap();
        for key in HOME_ENV_KEYS {
            env.set(key, home.path());
        }
    }

    /// A listener that accepts and then says nothing.
    ///
    /// The case the old per-candidate budget existed for, and the only one
    /// that can hold a round to `DISCOVERY_ROUND_TIMEOUT`: a port with nothing
    /// behind it refuses the connection outright and fails in microseconds.
    fn spawn_black_hole() -> u16 {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind black hole");
        let port = listener.local_addr().expect("black hole addr").port();
        std::thread::spawn(move || {
            // The streams are held rather than dropped: dropping one sends a
            // FIN, which lets the client fail fast -- the opposite of what this
            // fixture is for.
            let mut held = Vec::new();
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                held.push(stream);
            }
        });
        port
    }

    fn port_of(base_url: &str) -> u16 {
        base_url
            .rsplit(':')
            .next()
            .and_then(|port| port.parse().ok())
            .expect("the test server reports a port")
    }

    /// The Antigravity entry of the real provider registry.
    ///
    /// Taken from the registry rather than hand-assembled because the gate
    /// under test *is* that tuple's `has`: a provider it rejects never reaches
    /// a fetch. Driving `fetch_all_report_with_codex` instead would fan out to
    /// every other provider and read the developer's real credentials, so these
    /// tests stop at the one entry.
    fn registered_antigravity_provider() -> (fn() -> bool, Fetch) {
        let (_, has_credentials, fetch) = usage_providers(Fetch::Multi(fetch_all))
            .into_iter()
            .find(|(provider, _, _)| *provider == PROVIDER)
            .expect("Antigravity is a registered usage provider");
        (has_credentials, fetch)
    }

    /// What a failed availability assertion needs in order to be actionable.
    ///
    /// The gate is one `bool` and the ways it reaches `false` are far apart:
    /// the home redirect was lost, the log was not parsed, the request never
    /// went out, or it went out and was abandoned at the round deadline. A
    /// bare `assert!` separates none of them, so a CI failure here arrived
    /// with nothing to act on. `requests` is the sharpest of these -- zero
    /// means discovery never found the port, one means it asked and did not
    /// like the answer.
    fn discovery_state(started: std::time::Instant, seen: &Arc<Mutex<Vec<Seen>>>) -> String {
        let log = cli_log_path();
        format!(
            "elapsed={:?} home={:?} log={:?} log_exists={:?} parsed_ports={:?} requests={:?}",
            started.elapsed(),
            std::env::var_os("HOME"),
            log,
            log.as_ref().map(|path| path.exists()),
            ports_from_cli_log(),
            seen.lock().map(|seen| seen.len()),
        )
    }

    /// Regression for #1280.
    ///
    /// Antigravity only reaches the report if `has_credentials` says so, and
    /// that gate ran a whole quota request under a 400ms budget. A language
    /// server that answers slower than that -- which a loaded machine does
    /// routinely -- was filtered out of the provider list before any fetch
    /// ran, so no budget handed to `fetch_all` could have rescued it.
    ///
    /// The request count is the other half of the fix: discovery validates a
    /// port by asking it for quota, and `fetch_all` used to discard that answer
    /// and ask again.
    ///
    /// Two bounds are in play, not one. The fixture's delay is a floor on how
    /// slow the server is, and load can only push the answer later -- later is
    /// what the assertion wants. But `DISCOVERY_ROUND_TIMEOUT` is production
    /// code on this path and is a ceiling the test has to stay under, and the
    /// delay already spends a fifth of it before anything else happens. The
    /// diagnostics on the assertion exist so a failure says which of the two
    /// was hit.
    #[test]
    #[serial]
    fn a_language_server_slower_than_the_old_probe_budget_still_reports() {
        // Past the old 400ms budget by enough that scheduling noise cannot
        // close the gap, and a fifth of DISCOVERY_ROUND_TIMEOUT.
        const SERVER_DELAY: Duration = Duration::from_secs(1);

        let (base, seen) = spawn_server(|_path, _calls| {
            std::thread::sleep(SERVER_DELAY);
            (200, FIXTURE_QUOTA.to_string())
        });

        let home = TempDir::new().unwrap();
        let mut env = EnvGuard::capture(&HOME_ENV_KEYS);
        redirect_home_to_logged_ports(&mut env, &home, &[port_of(&base)]);

        let (has_credentials, fetch) = registered_antigravity_provider();
        let started = std::time::Instant::now();
        assert!(
            has_credentials(),
            "a language server answering in {SERVER_DELAY:?} must not be filtered out of \
             the report -- {}",
            discovery_state(started, &seen)
        );

        let started = std::time::Instant::now();
        let outputs = fetch.call().unwrap_or_else(|error| {
            panic!(
                "the provider the gate admitted must also fetch: {error} -- {}",
                discovery_state(started, &seen)
            )
        });
        assert_eq!(outputs.len(), 1, "one output per quota group");
        assert_eq!(outputs[0].provider, PROVIDER);
        assert_eq!(
            outputs[0].account.as_ref().unwrap().label.as_deref(),
            Some("Fixture Models"),
            "the row must come from the fixture, not from a language server \
             that happens to be running on this machine"
        );
        assert_eq!(outputs[0].metrics.len(), 1);
        assert!((outputs[0].metrics[0].used_percent - 75.0).abs() < 1e-6);

        assert_eq!(
            seen.lock().unwrap().len(),
            2,
            "one request for the availability gate and one for the fetch: the \
             fetch must reuse the summary discovery already validated"
        );
    }

    /// A candidate that accepts and then goes quiet must not decide the outcome
    /// for the candidates behind it.
    ///
    /// This is what makes one round deadline affordable where a budget per
    /// candidate was not: the requests are in flight together, so a stalled
    /// port costs a healthy one nothing. Asked in sequence under the same
    /// deadline, the two black holes would hold all of it and the language
    /// server listed behind them would never be reached.
    ///
    /// The server is ranked last on purpose, so each call also pays the whole
    /// `DISCOVERY_ROUND_TIMEOUT` waiting to see whether either stalled
    /// candidate -- both of which outrank it -- turns out to be merely slow.
    /// That wait is the cost of not attributing quota to a stale server, and it
    /// is what makes this test take seconds rather than milliseconds.
    #[test]
    #[serial]
    fn a_stalled_candidate_does_not_hide_the_language_server_behind_it() {
        let (base, seen) = spawn_server(|_path, _calls| (200, FIXTURE_QUOTA.to_string()));

        let home = TempDir::new().unwrap();
        let mut env = EnvGuard::capture(&HOME_ENV_KEYS);
        redirect_home_to_logged_ports(
            &mut env,
            &home,
            &[spawn_black_hole(), spawn_black_hole(), port_of(&base)],
        );

        let (has_credentials, fetch) = registered_antigravity_provider();
        let started = std::time::Instant::now();
        assert!(
            has_credentials(),
            "a stalled candidate must not consume the round the others share -- {}",
            discovery_state(started, &seen)
        );
        let started = std::time::Instant::now();
        let outputs = fetch.call().unwrap_or_else(|error| {
            panic!(
                "the fetch must reach the same server: {error} -- {}",
                discovery_state(started, &seen)
            )
        });
        assert_eq!(
            outputs[0].account.as_ref().unwrap().label.as_deref(),
            Some("Fixture Models")
        );
    }

    /// The newest logged port wins a round it did not finish first.
    ///
    /// [`ports_from_cli_log`] orders candidates newest first on purpose: the
    /// log is appended across runs, so the last entry is the current server
    /// and an earlier one can be a stale `agy` from a restart or an account
    /// switch. Asking them together throws that ordering away if the round
    /// resolves by completion order, which is how a signed-in user drew an
    /// "Antigravity is running but not signed in" error from a signed-out
    /// leftover -- an empty group list parses perfectly well.
    ///
    /// The ordering here is a handshake rather than a delay race: the newer
    /// server does not answer until the older one has been asked, so "the
    /// older finished first" holds however loaded the machine is. What follows
    /// the handshake is a delay only the round budget covers. An earlier
    /// revision stopped waiting for a better-ranked candidate 2s after the
    /// first answer, and with that grace in place this test reports the stale
    /// server: the current one answered inside the round and still lost.
    ///
    /// Two bounds again, as in the #1280 test above: the delay is a floor on
    /// how late the newer answer is, and load can only make it later, which
    /// is the direction the assertion wants -- but `DISCOVERY_ROUND_TIMEOUT` is
    /// the ceiling it has to stay under, with 2s to spare.
    #[test]
    #[serial]
    fn the_newest_logged_port_wins_even_when_an_older_one_answers_first() {
        const OLDER_QUOTA: &str =
            r#"{"response":{"groups":[{"displayName":"Stale Server","buckets":[]}]}}"#;
        // Past the 2s an earlier revision granted preference, so a grace that
        // short fails this; a fifth of the round short of its deadline.
        const NEWER_DELAY: Duration = Duration::from_secs(3);

        let (asked_older, older_was_asked) = std::sync::mpsc::channel::<()>();
        let (older_base, _older_seen) = spawn_server(move |_path, _calls| {
            let _ = asked_older.send(());
            (200, OLDER_QUOTA.to_string())
        });
        let (newer_base, newer_seen) = spawn_server(move |_path, _calls| {
            // A stall guard, not a budget: it only fires if the older server
            // was never reached, in which case the test has nothing to say and
            // should fail rather than hang.
            let _ = older_was_asked.recv_timeout(Duration::from_secs(30));
            std::thread::sleep(NEWER_DELAY);
            (200, FIXTURE_QUOTA.to_string())
        });

        let home = TempDir::new().unwrap();
        let mut env = EnvGuard::capture(&HOME_ENV_KEYS);
        redirect_home_to_logged_ports(
            &mut env,
            &home,
            &[port_of(&newer_base), port_of(&older_base)],
        );

        let (has_credentials, fetch) = registered_antigravity_provider();
        let started = std::time::Instant::now();
        assert!(
            has_credentials(),
            "two healthy candidates must still clear the gate -- {}",
            discovery_state(started, &newer_seen)
        );
        let started = std::time::Instant::now();
        let outputs = fetch.call().unwrap_or_else(|error| {
            panic!(
                "the fetch must reach a server: {error} -- {}",
                discovery_state(started, &newer_seen)
            )
        });
        assert_eq!(
            outputs[0].account.as_ref().unwrap().label.as_deref(),
            Some("Fixture Models"),
            "the round must return the newest logged port, not whichever candidate \
             answered first"
        );
    }

    /// A source that spends its whole round must not silence the next one.
    ///
    /// Discovery used to compute one deadline and hand it to both sources. A
    /// logged port that accepts and then goes quiet held it to expiry, and
    /// `tokio::time::timeout_at` on an expired deadline polls its inner future
    /// once and then the delay: the task set is dropped before any of its
    /// requests opens a socket. So the second source was enumerated, paid for
    /// and never asked -- another way for Antigravity to vanish from the
    /// report, which is the failure class this change exists to remove.
    ///
    /// The sources are fixtures because the real second source shells out to
    /// `ps`. What is under test is the sequencing, not what `ps` finds.
    #[test]
    fn a_source_that_spends_its_round_still_leaves_the_next_one_asked() {
        let stalled = spawn_black_hole();
        let (base, seen) = spawn_server(|_path, _calls| (200, FIXTURE_QUOTA.to_string()));
        let healthy = port_of(&base);
        let first: &dyn Fn() -> Vec<u16> = &|| vec![stalled];
        let second: &dyn Fn() -> Vec<u16> = &|| vec![healthy];

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let started = std::time::Instant::now();
        let summary = runtime.block_on(discover_quota_from(&[first, second]));
        let elapsed = started.elapsed();

        assert!(
            summary.is_some(),
            "the second source must still be asked after the first spent its round \
             (elapsed={elapsed:?}, requests={:?})",
            seen.lock().map(|seen| seen.len())
        );
        assert_eq!(
            seen.lock().unwrap().len(),
            1,
            "the healthy server must have been asked exactly once"
        );
        assert!(
            elapsed >= DISCOVERY_ROUND_TIMEOUT,
            "the first round is meant to have run to its deadline; if it did not, this \
             test no longer exercises a spent round (elapsed={elapsed:?})"
        );
    }

    /// A round with no candidates must not build a client.
    ///
    /// The client build is the expensive half of a round -- see
    /// [`quota_client`] -- and it used to run before discovery knew whether it
    /// had a single port to send to, so a machine without Antigravity paid for
    /// it on every `tokscale usage`, in the sequential pre-flight loop rather
    /// than the fan-out. The second half of this test is what stops the first
    /// half from passing vacuously.
    ///
    /// The count is kept by the builder this test hands in, so it holds exactly
    /// the builds these two calls caused: no other round, on this thread or
    /// any other, can reach it. Nothing here is `#[serial]` and nothing needs
    /// to be.
    #[test]
    fn a_round_with_no_candidates_builds_no_client() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        let builds = std::cell::Cell::new(0usize);
        let counted_client = || {
            builds.set(builds.get() + 1);
            quota_client()
        };

        assert!(runtime
            .block_on(race_for_quota_with(Vec::new(), &counted_client))
            .is_none());
        assert_eq!(
            builds.get(),
            0,
            "an empty candidate list must return before building anything"
        );

        let (base, _seen) = spawn_server(|_path, _calls| (200, FIXTURE_QUOTA.to_string()));
        assert!(runtime
            .block_on(race_for_quota_with(vec![port_of(&base)], &counted_client))
            .is_some());
        assert_eq!(
            builds.get(),
            1,
            "one candidate builds exactly one client, which is what makes the count \
             above meaningful"
        );
    }

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
    ///
    /// `#[serial]` because those two calls run the home-rooted discovery, so
    /// left to overlap they answer the fixture server of whichever test above
    /// currently has `HOME` redirected, and its request count then counts
    /// theirs.
    #[test]
    #[serial]
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
