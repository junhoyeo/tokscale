use anyhow::Result;
use chrono::{NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const APP_SERVER_TIMEOUT: Duration = Duration::from_secs(10);
const INITIALIZE_REQUEST_ID: i64 = 1;
const ACTIVITY_REQUEST_ID: i64 = 2;
const MAX_JSONL_LINE_BYTES: usize = 1024 * 1024;
const MAX_PENDING_FRAMES: usize = 16;
const APP_SERVER_SOURCE: &str = "codex-app-server";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CodexAccountActivityStatus {
    Available,
    UnsupportedCli,
    UnsupportedAuth,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexAccountActivitySnapshot {
    pub status: CodexAccountActivityStatus,
    pub source: &'static str,
    pub lifetime_tokens: Option<u64>,
    pub peak_daily_tokens: Option<u64>,
    pub longest_running_turn_sec: Option<u64>,
    pub current_streak_days: Option<u64>,
    pub longest_streak_days: Option<u64>,
    pub daily_usage_buckets: Option<Vec<CodexAccountActivityDailyBucket>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fetched_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexAccountActivityDailyBucket {
    pub start_date: String,
    pub tokens: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AppServerActivityResult {
    #[serde(default)]
    summary: Option<AppServerActivitySummary>,
    #[serde(default)]
    daily_usage_buckets: Option<Vec<AppServerDailyUsageBucket>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AppServerActivitySummary {
    lifetime_tokens: Option<u64>,
    peak_daily_tokens: Option<u64>,
    longest_running_turn_sec: Option<u64>,
    current_streak_days: Option<u64>,
    longest_streak_days: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AppServerDailyUsageBucket {
    start_date: String,
    tokens: u64,
}

#[derive(Debug)]
enum ActivityFetchError {
    UnsupportedCli,
    UnsupportedAuth,
    Unavailable(&'static str),
}

impl ActivityFetchError {
    fn snapshot(self) -> CodexAccountActivitySnapshot {
        let (status, message) = match self {
            Self::UnsupportedCli => (
                CodexAccountActivityStatus::UnsupportedCli,
                "The installed Codex CLI does not support account activity.".to_string(),
            ),
            Self::UnsupportedAuth => (
                CodexAccountActivityStatus::UnsupportedAuth,
                "Codex account activity requires supported Codex-service authentication."
                    .to_string(),
            ),
            Self::Unavailable(message) => (CodexAccountActivityStatus::Unavailable, message.into()),
        };

        CodexAccountActivitySnapshot {
            status,
            source: APP_SERVER_SOURCE,
            lifetime_tokens: None,
            peak_daily_tokens: None,
            longest_running_turn_sec: None,
            current_streak_days: None,
            longest_streak_days: None,
            daily_usage_buckets: None,
            fetched_at: None,
            message: Some(message),
        }
    }
}

enum AppServerFrame {
    Json(String),
    Oversized,
    InvalidUtf8,
}

struct AppServerTransport {
    child: Child,
    stdin: ChildStdin,
    frames: Option<mpsc::Receiver<AppServerFrame>>,
    stdout_reader: Option<JoinHandle<()>>,
    stderr_reader: Option<JoinHandle<()>>,
}

impl AppServerTransport {
    fn spawn() -> std::result::Result<Self, ActivityFetchError> {
        let mut child = Command::new("codex")
            .args(["app-server", "--stdio"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| match error.kind() {
                std::io::ErrorKind::NotFound => ActivityFetchError::UnsupportedCli,
                _ => ActivityFetchError::Unavailable("Could not start Codex app-server."),
            })?;

        let stdin = child.stdin.take();
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let (Some(stdin), Some(stdout), Some(stderr)) = (stdin, stdout, stderr) else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(ActivityFetchError::Unavailable(
                "Codex app-server did not expose its required standard streams.",
            ));
        };
        let (sender, frames) = mpsc::sync_channel(MAX_PENDING_FRAMES);

        Ok(Self {
            child,
            stdin,
            frames: Some(frames),
            stdout_reader: Some(spawn_stdout_reader(stdout, sender)),
            stderr_reader: Some(spawn_stderr_drain(stderr)),
        })
    }

    fn write_message(&mut self, message: &Value) -> std::result::Result<(), ActivityFetchError> {
        serde_json::to_writer(&mut self.stdin, message).map_err(|_| {
            ActivityFetchError::Unavailable("Could not encode Codex app-server input.")
        })?;
        self.stdin
            .write_all(b"\n")
            .and_then(|_| self.stdin.flush())
            .map_err(|_| ActivityFetchError::Unavailable("Codex app-server closed its input."))
    }

    fn wait_for_response(
        &mut self,
        expected_id: i64,
        deadline: Instant,
    ) -> std::result::Result<Value, ActivityFetchError> {
        loop {
            let remaining = deadline.checked_duration_since(Instant::now()).ok_or(
                ActivityFetchError::Unavailable("Timed out waiting for Codex account activity."),
            )?;
            let frame = self
                .frames
                .as_ref()
                .ok_or(ActivityFetchError::Unavailable(
                    "Codex app-server closed before returning account activity.",
                ))?
                .recv_timeout(remaining)
                .map_err(|error| match error {
                    mpsc::RecvTimeoutError::Timeout => ActivityFetchError::Unavailable(
                        "Timed out waiting for Codex account activity.",
                    ),
                    mpsc::RecvTimeoutError::Disconnected => ActivityFetchError::Unavailable(
                        "Codex app-server closed before returning account activity.",
                    ),
                })?;
            let line = match frame {
                AppServerFrame::Json(line) => line,
                AppServerFrame::Oversized => {
                    return Err(ActivityFetchError::Unavailable(
                        "Codex app-server returned an oversized protocol message.",
                    ));
                }
                AppServerFrame::InvalidUtf8 => {
                    return Err(ActivityFetchError::Unavailable(
                        "Codex app-server returned a non-text protocol message.",
                    ));
                }
            };
            if line.trim().is_empty() {
                continue;
            }
            let message = serde_json::from_str::<Value>(&line).map_err(|_| {
                ActivityFetchError::Unavailable(
                    "Codex app-server returned an invalid protocol message.",
                )
            })?;

            if is_server_request(&message) {
                self.write_message(&unsupported_server_request_response(&message))?;
                continue;
            }

            if message.get("id").and_then(Value::as_i64) == Some(expected_id) {
                return Ok(message);
            }
        }
    }
}

impl Drop for AppServerTransport {
    fn drop(&mut self) {
        // Drop the receiver before joining stdout so a blocked bounded sender can exit.
        self.frames.take();
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.stdout_reader.take() {
            let _ = reader.join();
        }
        if let Some(reader) = self.stderr_reader.take() {
            let _ = reader.join();
        }
    }
}

fn spawn_stdout_reader(
    stdout: ChildStdout,
    sender: mpsc::SyncSender<AppServerFrame>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        while let Ok(Some(frame)) = read_jsonl_frame(&mut reader) {
            if sender.send(frame).is_err() {
                return;
            }
        }
    })
}

fn spawn_stderr_drain(mut stderr: ChildStderr) -> JoinHandle<()> {
    thread::spawn(move || {
        let _ = std::io::copy(&mut stderr, &mut std::io::sink());
    })
}

fn read_jsonl_frame<R: BufRead>(reader: &mut R) -> std::io::Result<Option<AppServerFrame>> {
    let mut bytes = Vec::new();
    loop {
        let chunk = reader.fill_buf()?;
        if chunk.is_empty() {
            return if bytes.is_empty() {
                Ok(None)
            } else {
                Ok(Some(frame_from_bytes(bytes)))
            };
        }

        let newline = chunk.iter().position(|byte| *byte == b'\n');
        let content_len = newline.unwrap_or(chunk.len());
        if bytes.len().saturating_add(content_len) > MAX_JSONL_LINE_BYTES {
            let consumed = newline.map_or(chunk.len(), |index| index + 1);
            reader.consume(consumed);
            if newline.is_none() {
                discard_until_newline(reader)?;
            }
            return Ok(Some(AppServerFrame::Oversized));
        }

        bytes.extend_from_slice(&chunk[..content_len]);
        let consumed = newline.map_or(chunk.len(), |index| index + 1);
        reader.consume(consumed);
        if newline.is_some() {
            return Ok(Some(frame_from_bytes(bytes)));
        }
    }
}

fn discard_until_newline<R: BufRead>(reader: &mut R) -> std::io::Result<()> {
    loop {
        let chunk = reader.fill_buf()?;
        if chunk.is_empty() {
            return Ok(());
        }
        if let Some(index) = chunk.iter().position(|byte| *byte == b'\n') {
            reader.consume(index + 1);
            return Ok(());
        }
        let len = chunk.len();
        reader.consume(len);
    }
}

fn frame_from_bytes(bytes: Vec<u8>) -> AppServerFrame {
    match String::from_utf8(bytes) {
        Ok(line) => AppServerFrame::Json(line),
        Err(_) => AppServerFrame::InvalidUtf8,
    }
}

fn is_server_request(message: &Value) -> bool {
    message.get("method").and_then(Value::as_str).is_some()
        && message.get("id").is_some()
        && message.get("result").is_none()
        && message.get("error").is_none()
}

fn unsupported_server_request_response(message: &Value) -> Value {
    serde_json::json!({
        "id": message.get("id").cloned().unwrap_or(Value::Null),
        "error": {
            "code": -32601,
            "message": "Method not supported by tokscale"
        }
    })
}

fn initialize_request() -> Value {
    serde_json::json!({
        "id": INITIALIZE_REQUEST_ID,
        "method": "initialize",
        "params": {
            "clientInfo": {
                "name": "tokscale",
                "title": "Tokscale",
                "version": env!("CARGO_PKG_VERSION")
            }
        }
    })
}

fn initialized_notification() -> Value {
    serde_json::json!({
        "method": "initialized",
        "params": {}
    })
}

fn activity_request() -> Value {
    serde_json::json!({
        "id": ACTIVITY_REQUEST_ID,
        "method": "account/usage/read"
    })
}

fn fetch_activity_from_app_server(
) -> std::result::Result<CodexAccountActivitySnapshot, ActivityFetchError> {
    let mut transport = AppServerTransport::spawn()?;
    let deadline = Instant::now() + APP_SERVER_TIMEOUT;

    transport.write_message(&initialize_request())?;
    let initialize_response = transport.wait_for_response(INITIALIZE_REQUEST_ID, deadline)?;
    if initialize_response.get("error").is_some() {
        return Err(classify_rpc_error(&initialize_response));
    }
    if initialize_response.get("result").is_none() {
        return Err(ActivityFetchError::UnsupportedCli);
    }

    transport.write_message(&initialized_notification())?;
    transport.write_message(&activity_request())?;
    let activity_response = transport.wait_for_response(ACTIVITY_REQUEST_ID, deadline)?;
    if activity_response.get("error").is_some() {
        return Err(classify_rpc_error(&activity_response));
    }
    let result = activity_response
        .get("result")
        .ok_or(ActivityFetchError::Unavailable(
            "Codex app-server returned an invalid account activity response.",
        ))?;

    parse_activity_result(result)
}

fn classify_rpc_error(response: &Value) -> ActivityFetchError {
    let error = response.get("error");
    let code = error
        .and_then(|error| error.get("code"))
        .and_then(Value::as_i64);
    if code == Some(-32601) {
        return ActivityFetchError::UnsupportedCli;
    }

    let message = error
        .and_then(|error| error.get("message"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    if message.contains("auth")
        || message.contains("sign in")
        || message.contains("login")
        || message.contains("not authenticated")
    {
        return ActivityFetchError::UnsupportedAuth;
    }

    ActivityFetchError::Unavailable("Codex app-server could not read account activity.")
}

fn parse_activity_result(
    result: &Value,
) -> std::result::Result<CodexAccountActivitySnapshot, ActivityFetchError> {
    let activity: AppServerActivityResult =
        serde_json::from_value(result.clone()).map_err(|_| {
            ActivityFetchError::Unavailable(
                "Codex app-server returned an invalid account activity response.",
            )
        })?;
    let daily_usage_buckets = activity
        .daily_usage_buckets
        .map(|buckets| {
            buckets
                .into_iter()
                .map(|bucket| {
                    NaiveDate::parse_from_str(&bucket.start_date, "%Y-%m-%d").map_err(|_| {
                        ActivityFetchError::Unavailable(
                            "Codex app-server returned an invalid daily activity date.",
                        )
                    })?;
                    Ok(CodexAccountActivityDailyBucket {
                        start_date: bucket.start_date,
                        tokens: bucket.tokens,
                    })
                })
                .collect::<std::result::Result<Vec<_>, _>>()
        })
        .transpose()?;
    let summary = activity.summary;

    Ok(CodexAccountActivitySnapshot {
        status: CodexAccountActivityStatus::Available,
        source: APP_SERVER_SOURCE,
        lifetime_tokens: summary.as_ref().and_then(|summary| summary.lifetime_tokens),
        peak_daily_tokens: summary
            .as_ref()
            .and_then(|summary| summary.peak_daily_tokens),
        longest_running_turn_sec: summary
            .as_ref()
            .and_then(|summary| summary.longest_running_turn_sec),
        current_streak_days: summary
            .as_ref()
            .and_then(|summary| summary.current_streak_days),
        longest_streak_days: summary
            .as_ref()
            .and_then(|summary| summary.longest_streak_days),
        daily_usage_buckets,
        fetched_at: Some(Utc::now().to_rfc3339()),
        message: None,
    })
}

pub fn run(json: bool) -> Result<()> {
    let activity = match fetch_activity_from_app_server() {
        Ok(activity) => activity,
        Err(error) => error.snapshot(),
    };

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&activity_json(&activity))?
        );
        return Ok(());
    }

    render_activity(&activity);
    Ok(())
}

fn activity_json(activity: &CodexAccountActivitySnapshot) -> Value {
    serde_json::json!({
        "codexAccountActivity": activity,
    })
}

fn render_activity(activity: &CodexAccountActivitySnapshot) {
    use colored::Colorize;

    println!("\n  {}\n", "Codex - Account activity (supplemental)".cyan());
    println!(
        "  {}",
        format!("Source: {}", activity.source).bright_black()
    );
    println!("  {}", "Cost: N/A".bright_black());
    match activity.status {
        CodexAccountActivityStatus::Available => {
            if let Some(fetched_at) = &activity.fetched_at {
                println!("  {}", format!("Fetched: {fetched_at}").bright_black());
            }
            render_optional_count("Lifetime tokens", activity.lifetime_tokens);
            render_optional_count("Peak daily tokens", activity.peak_daily_tokens);
            render_optional_count("Longest turn (seconds)", activity.longest_running_turn_sec);
            render_optional_count("Current streak (days)", activity.current_streak_days);
            render_optional_count("Longest streak (days)", activity.longest_streak_days);
            if let Some(buckets) = &activity.daily_usage_buckets {
                if buckets.is_empty() {
                    println!("  {}", "Daily buckets: none returned".bright_black());
                } else {
                    println!("  {}", "Daily activity:".bright_black());
                    for bucket in buckets {
                        println!("    {}  {}", bucket.start_date, bucket.tokens);
                    }
                }
            }
        }
        _ => {
            if let Some(message) = &activity.message {
                println!("  {}", message.yellow());
            }
        }
    }
    println!(
        "{}\n",
        "  Not included in local totals, reports, exports, or submissions.".bright_black()
    );
}

fn render_optional_count(label: &str, value: Option<u64>) {
    use colored::Colorize;

    if let Some(value) = value {
        println!("  {}", format!("{label}: {value}").white());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_complete_activity_result() {
        let snapshot = parse_activity_result(&serde_json::json!({
            "summary": {
                "lifetimeTokens": 1234567,
                "peakDailyTokens": 45678,
                "longestRunningTurnSec": 540,
                "currentStreakDays": 8,
                "longestStreakDays": 14
            },
            "dailyUsageBuckets": [
                {"startDate": "2026-06-18", "tokens": 12345}
            ]
        }))
        .unwrap();

        assert_eq!(snapshot.status, CodexAccountActivityStatus::Available);
        assert_eq!(snapshot.source, APP_SERVER_SOURCE);
        assert_eq!(snapshot.lifetime_tokens, Some(1_234_567));
        assert_eq!(snapshot.peak_daily_tokens, Some(45_678));
        assert_eq!(
            snapshot.daily_usage_buckets,
            Some(vec![CodexAccountActivityDailyBucket {
                start_date: "2026-06-18".into(),
                tokens: 12_345,
            }])
        );
        assert!(snapshot.fetched_at.is_some());
    }

    #[test]
    fn parses_nullable_activity_result_without_inventing_totals() {
        let snapshot = parse_activity_result(&serde_json::json!({
            "summary": {
                "lifetimeTokens": null,
                "peakDailyTokens": null,
                "longestRunningTurnSec": null,
                "currentStreakDays": null,
                "longestStreakDays": null
            },
            "dailyUsageBuckets": null
        }))
        .unwrap();

        let json = serde_json::to_value(&snapshot).unwrap();
        assert_eq!(snapshot.status, CodexAccountActivityStatus::Available);
        assert!(snapshot.lifetime_tokens.is_none());
        assert!(snapshot.daily_usage_buckets.is_none());
        assert!(json.get("totalTokens").is_none());
        assert!(json.get("cost").is_none());
    }

    #[test]
    fn rejects_invalid_activity_values() {
        let negative = parse_activity_result(&serde_json::json!({
            "summary": {"lifetimeTokens": -1}
        }));
        assert!(matches!(negative, Err(ActivityFetchError::Unavailable(_))));

        let invalid_date = parse_activity_result(&serde_json::json!({
            "dailyUsageBuckets": [{"startDate": "not-a-date", "tokens": 1}]
        }));
        assert!(matches!(
            invalid_date,
            Err(ActivityFetchError::Unavailable(_))
        ));
    }

    #[test]
    fn classifies_rpc_errors_without_exposing_server_text() {
        assert!(matches!(
            classify_rpc_error(&serde_json::json!({
                "error": {"code": -32601, "message": "Method not found"}
            })),
            ActivityFetchError::UnsupportedCli
        ));
        assert!(matches!(
            classify_rpc_error(&serde_json::json!({
                "error": {"code": -32000, "message": "Not authenticated"}
            })),
            ActivityFetchError::UnsupportedAuth
        ));
        let snapshot = classify_rpc_error(&serde_json::json!({
            "error": {"code": -32000, "message": "private detail"}
        }))
        .snapshot();
        assert_eq!(
            snapshot.message.as_deref(),
            Some("Codex app-server could not read account activity.")
        );
    }

    #[test]
    fn unavailable_snapshot_is_stable_and_has_no_fetch_time() {
        let snapshot = ActivityFetchError::UnsupportedCli.snapshot();
        let json = serde_json::to_value(&snapshot).unwrap();

        assert_eq!(json["status"], "unsupportedCli");
        assert_eq!(json["source"], APP_SERVER_SOURCE);
        assert!(json.get("fetchedAt").is_none());
        assert!(json.get("cost").is_none());
        assert!(json.get("totalTokens").is_none());
    }

    #[test]
    fn activity_json_uses_a_standalone_wrapper() {
        let snapshot = ActivityFetchError::UnsupportedCli.snapshot();
        let json = activity_json(&snapshot);

        assert_eq!(json["codexAccountActivity"]["status"], "unsupportedCli");
        assert!(json.get("totalTokens").is_none());
        assert!(json.get("cost").is_none());
    }

    #[test]
    fn protocol_messages_follow_the_required_handshake() {
        let initialize = initialize_request();
        let initialized = initialized_notification();
        let activity = activity_request();

        assert_eq!(initialize.get("id").and_then(Value::as_i64), Some(1));
        assert_eq!(initialize["method"], "initialize");
        assert_eq!(initialized["method"], "initialized");
        assert!(initialized.get("id").is_none());
        assert_eq!(activity.get("id").and_then(Value::as_i64), Some(2));
        assert_eq!(activity["method"], "account/usage/read");
        assert!(initialize.get("jsonrpc").is_none());
    }

    #[test]
    fn recognizes_and_rejects_server_requests() {
        let request = serde_json::json!({"id": 9, "method": "attestation/generate"});

        assert!(is_server_request(&request));
        assert_eq!(
            unsupported_server_request_response(&request),
            serde_json::json!({
                "id": 9,
                "error": {"code": -32601, "message": "Method not supported by tokscale"}
            })
        );
    }

    #[test]
    fn rejects_oversized_jsonl_frames() {
        let oversized = vec![b'x'; MAX_JSONL_LINE_BYTES + 1];
        let mut input = oversized;
        input.push(b'\n');
        let mut reader = BufReader::new(input.as_slice());

        assert!(matches!(
            read_jsonl_frame(&mut reader).unwrap(),
            Some(AppServerFrame::Oversized)
        ));
    }
}
