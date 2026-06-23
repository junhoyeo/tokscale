use anyhow::Result;

#[derive(Debug, Clone)]
pub(crate) enum CodexLoginOutcome {
    Imported(crate::commands::usage::codex::CodexAccountInfo),
    Failed(String),
}

#[derive(Debug)]
pub(crate) enum CodexLoginEvent {
    Output(String),
    Finished(CodexLoginOutcome),
}

/// Shared handle to the spawned `codex login` child process. The login worker
/// polls it via `try_wait`; the TUI marks the slot cancelled and takes the
/// child out to kill it on dismiss or exit.
pub(crate) type CodexLoginChildSlot = std::sync::Arc<std::sync::Mutex<CodexLoginChildState>>;

#[derive(Default)]
pub(crate) struct CodexLoginChildState {
    child: Option<std::process::Child>,
    cancelled: bool,
}

pub(crate) fn cancel_codex_login_child(slot: &CodexLoginChildSlot) {
    let child = slot.lock().ok().and_then(|mut state| {
        state.cancelled = true;
        state.child.take()
    });
    if let Some(mut child) = child {
        let _ = child.kill();
        let _ = child.wait();
    }
}

pub(crate) fn run_codex_login_worker(
    tx: std::sync::mpsc::Sender<CodexLoginEvent>,
    child_slot: CodexLoginChildSlot,
) {
    let result = run_codex_login_worker_inner(tx.clone(), child_slot);
    let outcome = match result {
        Ok(info) => CodexLoginOutcome::Imported(info),
        Err(e) => CodexLoginOutcome::Failed(e.to_string()),
    };
    let _ = tx.send(CodexLoginEvent::Finished(outcome));
}

fn run_codex_login_worker_inner(
    tx: std::sync::mpsc::Sender<CodexLoginEvent>,
    child_slot: CodexLoginChildSlot,
) -> Result<crate::commands::usage::codex::CodexAccountInfo> {
    let codex_home =
        std::env::temp_dir().join(format!("tokscale-codex-login-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&codex_home)
        .map_err(|e| anyhow::anyhow!("failed to create temporary Codex home: {e}"))?;

    let result = run_codex_login_in_home(&codex_home, tx, child_slot);
    let _ = std::fs::remove_dir_all(&codex_home);
    result
}

fn run_codex_login_in_home(
    codex_home: &std::path::Path,
    tx: std::sync::mpsc::Sender<CodexLoginEvent>,
    child_slot: CodexLoginChildSlot,
) -> Result<crate::commands::usage::codex::CodexAccountInfo> {
    let _ = tx.send(CodexLoginEvent::Output(
        "Starting Codex browser login".to_string(),
    ));

    let mut child = std::process::Command::new("codex")
        .arg("login")
        .env("CODEX_HOME", codex_home)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| anyhow::anyhow!("failed to start codex login: {e}"))?;

    let output_lines = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut readers = Vec::new();
    if let Some(stdout) = child.stdout.take() {
        readers.push(spawn_codex_login_output_reader(
            stdout,
            tx.clone(),
            std::sync::Arc::clone(&output_lines),
        ));
    }
    if let Some(stderr) = child.stderr.take() {
        readers.push(spawn_codex_login_output_reader(
            stderr,
            tx.clone(),
            std::sync::Arc::clone(&output_lines),
        ));
    }

    if let Some(mut cancelled_child) = put_codex_login_child(&child_slot, child)? {
        let _ = cancelled_child.kill();
        let _ = cancelled_child.wait();
        for reader in readers {
            let _ = reader.join();
        }
        anyhow::bail!("Codex login cancelled");
    }

    let status = wait_for_codex_login_child(&child_slot);
    for reader in readers {
        let _ = reader.join();
    }
    let Some(status) = status? else {
        // The TUI emptied the slot: the login was dismissed or the app exited.
        anyhow::bail!("Codex login cancelled");
    };

    if !status.success() {
        let output_lines = output_lines
            .lock()
            .map(|lines| lines.clone())
            .unwrap_or_default();
        anyhow::bail!("{}", codex_login_failure_message(&status, &output_lines));
    }

    let auth_path = codex_home.join("auth.json");
    let import = crate::commands::usage::codex::import_login_auth_file(&auth_path)?;
    if let Some(warning) = import.warning {
        let _ = tx.send(CodexLoginEvent::Output(warning));
    }
    Ok(import.info)
}

fn put_codex_login_child(
    child_slot: &CodexLoginChildSlot,
    child: std::process::Child,
) -> Result<Option<std::process::Child>> {
    let mut state = child_slot
        .lock()
        .map_err(|_| anyhow::anyhow!("codex login state lock poisoned"))?;
    if state.cancelled {
        Ok(Some(child))
    } else {
        state.child = Some(child);
        Ok(None)
    }
}

/// Polls the login child until it exits. Returns `Ok(None)` when the TUI
/// cancelled the login on dismiss or app exit.
fn wait_for_codex_login_child(
    child_slot: &CodexLoginChildSlot,
) -> Result<Option<std::process::ExitStatus>> {
    loop {
        {
            let mut state = child_slot
                .lock()
                .map_err(|_| anyhow::anyhow!("codex login state lock poisoned"))?;
            if state.cancelled {
                return Ok(None);
            }
            let Some(child) = state.child.as_mut() else {
                return Ok(None);
            };
            match child.try_wait() {
                Ok(Some(status)) => {
                    state.child = None;
                    return Ok(Some(status));
                }
                Ok(None) => {}
                Err(e) => {
                    state.child = None;
                    return Err(anyhow::anyhow!("failed to wait for codex login: {e}"));
                }
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(150));
    }
}

fn spawn_codex_login_output_reader<R>(
    reader: R,
    tx: std::sync::mpsc::Sender<CodexLoginEvent>,
    output_lines: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
) -> std::thread::JoinHandle<()>
where
    R: std::io::Read + Send + 'static,
{
    std::thread::spawn(move || {
        let reader = std::io::BufReader::new(reader);
        for line in std::io::BufRead::lines(reader).map_while(std::result::Result::ok) {
            let line = sanitize_codex_login_line(&line);
            if !line.trim().is_empty() {
                if let Ok(mut output_lines) = output_lines.lock() {
                    output_lines.push(line.clone());
                }
                let _ = tx.send(CodexLoginEvent::Output(line));
            }
        }
    })
}

fn codex_login_failure_message(
    status: &std::process::ExitStatus,
    output_lines: &[String],
) -> String {
    codex_login_failure_message_from_output(&status.to_string(), output_lines)
}

fn codex_login_failure_message_from_output(status: &str, output_lines: &[String]) -> String {
    let output = output_lines.join("\n").to_lowercase();

    if output.contains("429") || output.contains("too many requests") {
        return "OpenAI login is rate-limited (429 Too Many Requests). Wait before trying Add Codex again.".to_string();
    }

    if output.contains("expired") {
        return "Codex device code expired. Start Add Codex again to get a new code.".to_string();
    }

    if output.contains("device auth failed") {
        return "Codex device login failed. Try Add Codex again later.".to_string();
    }

    format!("codex login exited with {status}")
}

fn sanitize_codex_login_line(line: &str) -> String {
    let mut sanitized = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            match chars.next() {
                Some('[') => {
                    for ch in chars.by_ref() {
                        if ('\u{40}'..='\u{7e}').contains(&ch) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    while let Some(ch) = chars.next() {
                        if ch == '\x07' {
                            break;
                        }
                        if ch == '\x1b' && chars.peek() == Some(&'\\') {
                            let _ = chars.next();
                            break;
                        }
                    }
                }
                Some(_) | None => {}
            }
            continue;
        }

        if !ch.is_control() || ch == '\t' {
            sanitized.push(ch);
        }
    }

    sanitized
}

#[cfg(test)]
pub(crate) fn put_codex_login_child_for_test(
    child_slot: &CodexLoginChildSlot,
    child: std::process::Child,
) -> Result<Option<std::process::Child>> {
    put_codex_login_child(child_slot, child)
}

#[cfg(test)]
pub(crate) fn codex_login_slot_child_is_none_for_test(child_slot: &CodexLoginChildSlot) -> bool {
    child_slot
        .lock()
        .map(|state| state.child.is_none())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wait_for_codex_login_child_returns_none_when_cancelled() {
        let slot = CodexLoginChildSlot::default();
        cancel_codex_login_child(&slot);
        let status = wait_for_codex_login_child(&slot).unwrap();
        assert!(status.is_none());
    }

    #[cfg(unix)]
    #[test]
    fn put_codex_login_child_returns_child_when_already_cancelled() {
        let slot = CodexLoginChildSlot::default();
        cancel_codex_login_child(&slot);
        let child = std::process::Command::new("true").spawn().unwrap();

        let mut child = put_codex_login_child(&slot, child)
            .unwrap()
            .expect("cancelled slot should return the child to be killed by caller");

        assert!(child.wait().unwrap().success());
    }

    #[cfg(unix)]
    #[test]
    fn wait_for_codex_login_child_reports_exit_status() {
        let child = std::process::Command::new("true").spawn().unwrap();
        let slot = CodexLoginChildSlot::default();
        put_codex_login_child(&slot, child).unwrap();

        let status = wait_for_codex_login_child(&slot).unwrap();

        assert!(status.unwrap().success());
        assert!(slot.lock().unwrap().child.is_none());
    }

    #[test]
    fn test_sanitize_codex_login_line_strips_ansi_sequences() {
        assert_eq!(
            sanitize_codex_login_line("\u{1b}[94mhttps://auth.openai.com/codex/device\u{1b}[0m"),
            "https://auth.openai.com/codex/device"
        );
        assert_eq!(
            sanitize_codex_login_line("\u{1b}[90mCAGW-LNUYX\u{1b}[0m"),
            "CAGW-LNUYX"
        );
    }

    #[test]
    fn test_codex_login_failure_message_identifies_rate_limit() {
        let message = codex_login_failure_message_from_output(
            "exit status: 1",
            &[
                "Device codes are a common phishing target. Never share this code.".to_string(),
                "Error logging in with device code: device auth failed with status 429 Too Many Requests"
                    .to_string(),
            ],
        );

        assert_eq!(
            message,
            "OpenAI login is rate-limited (429 Too Many Requests). Wait before trying Add Codex again."
        );
    }

    #[test]
    fn test_codex_login_failure_message_identifies_expired_code() {
        let message = codex_login_failure_message_from_output(
            "exit status: 1",
            &["Error logging in with device code: expired".to_string()],
        );

        assert_eq!(
            message,
            "Codex device code expired. Start Add Codex again to get a new code."
        );
    }
}
