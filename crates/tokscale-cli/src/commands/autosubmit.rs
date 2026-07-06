use crate::tui::settings::{
    AutosubmitSettings, DEFAULT_AUTOSUBMIT_INTERVAL_MINUTES, MAX_AUTOSUBMIT_INTERVAL_MINUTES,
    MIN_AUTOSUBMIT_INTERVAL_MINUTES,
};
use crate::{ClientFlags, DateRangeFlags};
use anyhow::{bail, Context, Result};
use clap::{Args, Subcommand, ValueEnum};
use fs2::FileExt;
use serde::Serialize;
use std::fs::{self, OpenOptions};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::Command;

const JOB_ID: &str = "ai.tokscale.autosubmit";
const CRON_MARKER_BEGIN: &str = "# BEGIN TOKSCALE AUTOSUBMIT";
const CRON_MARKER_END: &str = "# END TOKSCALE AUTOSUBMIT";
const SKIP_SCHEDULER_ENV: &str = "TOKSCALE_AUTOSUBMIT_SKIP_SCHEDULER";

#[derive(Subcommand)]
pub enum AutosubmitSubcommand {
    #[command(about = "Enable periodic submit using the OS scheduler")]
    Enable(AutosubmitEnableArgs),
    #[command(about = "Show autosubmit status")]
    Status {
        #[arg(long, help = "Output as JSON")]
        json: bool,
    },
    #[command(about = "Disable autosubmit and remove its scheduler entry")]
    Disable,
    #[command(about = "Run autosubmit once if it is due")]
    Run {
        #[arg(long, help = "Run even when the configured interval has not elapsed")]
        force: bool,
    },
}

#[derive(Args)]
pub struct AutosubmitEnableArgs {
    #[arg(
        long,
        value_name = "DURATION",
        default_value = "24h",
        help = "Submit interval, e.g. 30m, 2h, or 1d"
    )]
    interval: String,
    #[command(flatten)]
    clients: ClientFlags,
    #[command(flatten)]
    date: DateRangeFlags,
    #[arg(long, value_enum, help = "Override the detected scheduler backend")]
    scheduler: Option<SchedulerKind>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum SchedulerKind {
    Launchd,
    Systemd,
    Cron,
    WindowsTaskScheduler,
}

impl SchedulerKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Launchd => "launchd",
            Self::Systemd => "systemd",
            Self::Cron => "cron",
            Self::WindowsTaskScheduler => "windows-task-scheduler",
        }
    }

    fn from_str(value: &str) -> Option<Self> {
        match value {
            "launchd" => Some(Self::Launchd),
            "systemd" => Some(Self::Systemd),
            "cron" => Some(Self::Cron),
            "windows-task-scheduler" => Some(Self::WindowsTaskScheduler),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutosubmitRunDecision {
    Disabled,
    NotDue { next_run_at_ms: i64 },
    Due,
}

pub struct AutosubmitRunLock {
    _file: std::fs::File,
}

#[derive(Debug, Clone)]
struct SchedulerSpec {
    files: Vec<(PathBuf, String)>,
    install_commands: Vec<(String, Vec<String>)>,
    uninstall_commands: Vec<(String, Vec<String>)>,
    cron_block: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct StatusOutput {
    enabled: bool,
    interval_minutes: u64,
    scheduler: Option<String>,
    clients: Vec<String>,
    since: Option<String>,
    until: Option<String>,
    year: Option<String>,
    today: bool,
    yesterday: bool,
    week: bool,
    month: bool,
    last_run_at_ms: Option<i64>,
    last_error: Option<String>,
}

pub fn enable(args: AutosubmitEnableArgs) -> Result<()> {
    let interval_minutes = parse_interval_minutes(&args.interval)?;
    let scheduler = args.scheduler.unwrap_or_else(default_scheduler_kind);
    let exe = std::env::current_exe().context("Could not resolve current tokscale executable")?;
    validate_scheduler_executable(&exe)?;

    let mut settings = crate::tui::settings::Settings::load();
    settings.autosubmit = AutosubmitSettings {
        enabled: true,
        interval_minutes,
        clients: clients_for_settings(args.clients),
        since: args.date.since,
        until: args.date.until,
        year: args.date.year,
        today: args.date.today,
        yesterday: args.date.yesterday,
        week: args.date.week,
        month: args.date.month,
        scheduler: Some(scheduler.as_str().to_string()),
        last_run_at_ms: settings.autosubmit.last_run_at_ms,
        last_error: None,
    };

    if !skip_scheduler_install() {
        install_scheduler(scheduler, &exe, &settings.autosubmit)?;
    }
    settings.save()?;

    println!(
        "Autosubmit enabled: every {} minutes via {}.",
        interval_minutes,
        scheduler.as_str()
    );
    Ok(())
}

pub fn status(json: bool) -> Result<()> {
    let autosubmit = crate::tui::settings::Settings::load().autosubmit;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&status_output(&autosubmit))?
        );
        return Ok(());
    }

    if autosubmit.enabled {
        println!("Autosubmit is enabled.");
        println!("  Interval: {} minutes", autosubmit.interval_minutes);
        println!(
            "  Scheduler: {}",
            autosubmit.scheduler.as_deref().unwrap_or("unknown")
        );
        if !autosubmit.clients.is_empty() {
            println!("  Clients: {}", autosubmit.clients.join(", "));
        } else {
            println!("  Clients: default submit clients");
        }
    } else {
        println!("Autosubmit is disabled.");
    }
    if let Some(last_run_at_ms) = autosubmit.last_run_at_ms {
        println!("  Last run: {}", format_timestamp_ms(last_run_at_ms));
    }
    if let Some(error) = autosubmit.last_error {
        println!("  Last error: {error}");
    }
    Ok(())
}

pub fn disable() -> Result<()> {
    let mut settings = crate::tui::settings::Settings::load();
    let scheduler = settings
        .autosubmit
        .scheduler
        .as_deref()
        .and_then(SchedulerKind::from_str)
        .unwrap_or_else(default_scheduler_kind);

    if settings.autosubmit.enabled && !skip_scheduler_install() {
        uninstall_scheduler(scheduler)?;
    }

    settings.autosubmit.enabled = false;
    settings.autosubmit.last_error = None;
    settings.save()?;
    println!("Autosubmit disabled.");
    Ok(())
}

pub fn load_run_config(
    force: bool,
    now_ms: i64,
) -> Result<(AutosubmitSettings, AutosubmitRunDecision)> {
    let settings = crate::tui::settings::Settings::load().autosubmit;
    let decision = run_decision(&settings, now_ms, force);
    Ok((settings, decision))
}

pub fn record_run_success(now_ms: i64) -> Result<()> {
    let mut settings = crate::tui::settings::Settings::load();
    settings.autosubmit.last_run_at_ms = Some(now_ms);
    settings.autosubmit.last_error = None;
    settings.save()
}

pub fn record_run_error(error: &str) -> Result<()> {
    let mut settings = crate::tui::settings::Settings::load();
    settings.autosubmit.last_error = Some(error.to_string());
    settings.save()
}

pub fn submit_filters(
    settings: &AutosubmitSettings,
) -> (
    Option<Vec<String>>,
    Option<String>,
    Option<String>,
    Option<String>,
) {
    let clients = if settings.clients.is_empty() {
        Some(default_submit_clients())
    } else {
        Some(settings.clients.clone())
    };
    let date = DateRangeFlags {
        today: settings.today,
        yesterday: settings.yesterday,
        week: settings.week,
        month: settings.month,
        since: settings.since.clone(),
        until: settings.until.clone(),
        year: settings.year.clone(),
    };
    let (since, until) = build_date_filter_for_date(&date, chrono::Local::now().date_naive());
    let year = if date.today || date.yesterday || date.week || date.month {
        None
    } else {
        date.year
    };
    (clients, since, until, year)
}

pub fn try_acquire_run_lock() -> Result<Option<AutosubmitRunLock>> {
    let path = autosubmit_lock_path()?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .with_context(|| format!("Could not open autosubmit lock at {}", path.display()))?;
    match file.try_lock_exclusive() {
        Ok(()) => Ok(Some(AutosubmitRunLock { _file: file })),
        Err(err) if err.kind() == ErrorKind::WouldBlock => Ok(None),
        Err(err) => Err(err)
            .with_context(|| format!("Could not lock autosubmit state at {}", path.display())),
    }
}

fn status_output(settings: &AutosubmitSettings) -> StatusOutput {
    StatusOutput {
        enabled: settings.enabled,
        interval_minutes: settings.interval_minutes,
        scheduler: settings.scheduler.clone(),
        clients: settings.clients.clone(),
        since: settings.since.clone(),
        until: settings.until.clone(),
        year: settings.year.clone(),
        today: settings.today,
        yesterday: settings.yesterday,
        week: settings.week,
        month: settings.month,
        last_run_at_ms: settings.last_run_at_ms,
        last_error: settings.last_error.clone(),
    }
}

pub fn run_decision(
    settings: &AutosubmitSettings,
    now_ms: i64,
    force: bool,
) -> AutosubmitRunDecision {
    if !settings.enabled {
        return AutosubmitRunDecision::Disabled;
    }
    if force {
        return AutosubmitRunDecision::Due;
    }
    let interval_ms = (settings.interval_minutes as i64).saturating_mul(60_000);
    match settings.last_run_at_ms {
        Some(last) if now_ms < last.saturating_add(interval_ms) => AutosubmitRunDecision::NotDue {
            next_run_at_ms: last.saturating_add(interval_ms),
        },
        _ => AutosubmitRunDecision::Due,
    }
}

pub fn parse_interval_minutes(input: &str) -> Result<u64> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        bail!("Interval cannot be empty");
    }
    let split = trimmed
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(trimmed.len());
    let (number, unit) = trimmed.split_at(split);
    let amount: u64 = number
        .parse()
        .with_context(|| format!("Invalid interval: {input}"))?;
    let multiplier = match unit.trim().to_ascii_lowercase().as_str() {
        "" | "m" | "min" | "mins" | "minute" | "minutes" => 1,
        "h" | "hr" | "hrs" | "hour" | "hours" => 60,
        "d" | "day" | "days" => 24 * 60,
        _ => bail!("Unsupported interval unit: {unit}"),
    };
    let minutes = amount
        .checked_mul(multiplier)
        .ok_or_else(|| anyhow::anyhow!("Interval is too large"))?;
    if !(MIN_AUTOSUBMIT_INTERVAL_MINUTES..=MAX_AUTOSUBMIT_INTERVAL_MINUTES).contains(&minutes) {
        bail!(
            "Interval must be between {} and {} minutes",
            MIN_AUTOSUBMIT_INTERVAL_MINUTES,
            MAX_AUTOSUBMIT_INTERVAL_MINUTES
        );
    }
    Ok(minutes)
}

fn clients_for_settings(flags: ClientFlags) -> Vec<String> {
    if flags.clients.is_empty() {
        return Vec::new();
    }
    let mut seen = std::collections::HashSet::new();
    flags
        .clients
        .into_iter()
        .map(|client| client.as_filter_str().to_string())
        .filter(|client| seen.insert(client.clone()))
        .collect()
}

fn default_submit_clients() -> Vec<String> {
    let mut clients: Vec<String> = tokscale_core::ClientId::iter()
        .filter(|client| client.submit_default())
        .map(|client| client.as_str().to_string())
        .collect();
    clients.push("synthetic".to_string());
    clients
}

fn skip_scheduler_install() -> bool {
    std::env::var(SKIP_SCHEDULER_ENV)
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false)
}

fn default_scheduler_kind() -> SchedulerKind {
    if cfg!(target_os = "macos") {
        SchedulerKind::Launchd
    } else if cfg!(target_os = "windows") {
        SchedulerKind::WindowsTaskScheduler
    } else if Command::new("systemctl")
        .args(["--user", "--version"])
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
    {
        SchedulerKind::Systemd
    } else {
        SchedulerKind::Cron
    }
}

fn install_scheduler(
    scheduler: SchedulerKind,
    exe: &Path,
    settings: &AutosubmitSettings,
) -> Result<()> {
    let spec = render_scheduler_spec(scheduler, exe, settings)?;
    for (path, content) in spec.files {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, content)?;
    }
    for (program, args) in spec.install_commands {
        run_status_command(&program, &args)?;
    }
    if let Some(block) = spec.cron_block {
        install_cron_block(&block)?;
    }
    Ok(())
}

fn uninstall_scheduler(scheduler: SchedulerKind) -> Result<()> {
    let dummy = AutosubmitSettings {
        interval_minutes: DEFAULT_AUTOSUBMIT_INTERVAL_MINUTES,
        ..AutosubmitSettings::default()
    };
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("tokscale"));
    let spec = render_scheduler_spec(scheduler, &exe, &dummy)?;
    for (program, args) in spec.uninstall_commands {
        let _ = Command::new(&program).args(&args).status();
    }
    if scheduler == SchedulerKind::Cron {
        let _ = uninstall_cron_block();
    }
    for (path, _) in spec.files {
        let _ = fs::remove_file(path);
    }
    Ok(())
}

fn render_scheduler_spec(
    scheduler: SchedulerKind,
    exe: &Path,
    settings: &AutosubmitSettings,
) -> Result<SchedulerSpec> {
    validate_scheduler_executable(exe)?;
    match scheduler {
        SchedulerKind::Launchd => render_launchd_spec(exe, settings),
        SchedulerKind::Systemd => render_systemd_spec(exe, settings),
        SchedulerKind::Cron => render_cron_spec(exe, settings),
        SchedulerKind::WindowsTaskScheduler => render_windows_task_spec(exe, settings),
    }
}

fn render_launchd_spec(exe: &Path, settings: &AutosubmitSettings) -> Result<SchedulerSpec> {
    let home = dirs::home_dir().context("Could not determine home directory")?;
    let plist_path = home
        .join("Library")
        .join("LaunchAgents")
        .join(format!("{JOB_ID}.plist"));
    let log_path = autosubmit_log_path()?;
    let interval_seconds = settings.interval_minutes.saturating_mul(60).max(60);
    let content = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "https://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>{job}</string>
  <key>ProgramArguments</key>
  <array>
    <string>{exe}</string>
    <string>autosubmit</string>
    <string>run</string>
  </array>
  <key>RunAtLoad</key><true/>
  <key>StartInterval</key><integer>{interval}</integer>
  <key>StandardOutPath</key><string>{log}</string>
  <key>StandardErrorPath</key><string>{log}</string>
</dict>
</plist>
"#,
        job = xml_escape(JOB_ID),
        exe = xml_escape(&exe.to_string_lossy()),
        interval = interval_seconds,
        log = xml_escape(&log_path.to_string_lossy())
    );
    Ok(SchedulerSpec {
        files: vec![(plist_path.clone(), content)],
        cron_block: None,
        install_commands: vec![(
            "launchctl".to_string(),
            vec![
                "load".to_string(),
                plist_path.to_string_lossy().into_owned(),
            ],
        )],
        uninstall_commands: vec![(
            "launchctl".to_string(),
            vec![
                "unload".to_string(),
                plist_path.to_string_lossy().into_owned(),
            ],
        )],
    })
}

fn render_systemd_spec(exe: &Path, settings: &AutosubmitSettings) -> Result<SchedulerSpec> {
    let user_dir = systemd_user_dir()?;
    let service_path = user_dir.join("tokscale-autosubmit.service");
    let timer_path = user_dir.join("tokscale-autosubmit.timer");
    let log_path = autosubmit_log_path()?;
    let service = format!(
        "[Unit]\nDescription=Tokscale autosubmit\n\n[Service]\nType=oneshot\nExecStart={} autosubmit run\nStandardOutput=append:{}\nStandardError=append:{}\n",
        systemd_escape_path(exe),
        systemd_escape_path(&log_path),
        systemd_escape_path(&log_path)
    );
    let timer = format!(
        "[Unit]\nDescription=Run Tokscale autosubmit periodically\n\n[Timer]\nOnBootSec=5m\nOnUnitActiveSec={}min\nPersistent=true\n\n[Install]\nWantedBy=timers.target\n",
        settings.interval_minutes
    );
    Ok(SchedulerSpec {
        files: vec![(service_path, service), (timer_path, timer)],
        cron_block: None,
        install_commands: vec![
            (
                "systemctl".to_string(),
                vec!["--user".to_string(), "daemon-reload".to_string()],
            ),
            (
                "systemctl".to_string(),
                vec![
                    "--user".to_string(),
                    "enable".to_string(),
                    "--now".to_string(),
                    "tokscale-autosubmit.timer".to_string(),
                ],
            ),
        ],
        uninstall_commands: vec![
            (
                "systemctl".to_string(),
                vec![
                    "--user".to_string(),
                    "disable".to_string(),
                    "--now".to_string(),
                    "tokscale-autosubmit.timer".to_string(),
                ],
            ),
            (
                "systemctl".to_string(),
                vec!["--user".to_string(), "daemon-reload".to_string()],
            ),
        ],
    })
}

fn systemd_user_dir() -> Result<PathBuf> {
    let config_dir = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(dirs::config_dir)
        .or_else(|| dirs::home_dir().map(|home| home.join(".config")));
    Ok(config_dir
        .context("Could not determine XDG config directory")?
        .join("systemd")
        .join("user"))
}

fn render_cron_spec(exe: &Path, settings: &AutosubmitSettings) -> Result<SchedulerSpec> {
    let log_path = autosubmit_log_path()?;
    let interval = settings.interval_minutes.max(1);
    let schedule = if interval < 60 {
        format!("*/{interval} * * * *")
    } else {
        "0 * * * *".to_string()
    };
    let line = format!(
        "{schedule} {} autosubmit run >> {} 2>&1",
        shell_quote(&exe.to_string_lossy()),
        shell_quote(&log_path.to_string_lossy())
    );
    let block = format!("{CRON_MARKER_BEGIN}\n{line}\n{CRON_MARKER_END}");
    Ok(SchedulerSpec {
        files: Vec::new(),
        install_commands: Vec::new(),
        uninstall_commands: Vec::new(),
        cron_block: Some(block),
    })
}

fn render_windows_task_spec(exe: &Path, settings: &AutosubmitSettings) -> Result<SchedulerSpec> {
    let (schedule, modifier) = windows_schedule(settings.interval_minutes)?;
    let task = format!(r#""{}" autosubmit run"#, exe.display());
    Ok(SchedulerSpec {
        files: Vec::new(),
        cron_block: None,
        install_commands: vec![(
            "schtasks".to_string(),
            vec![
                "/Create".to_string(),
                "/F".to_string(),
                "/SC".to_string(),
                schedule,
                "/MO".to_string(),
                modifier,
                "/TN".to_string(),
                JOB_ID.to_string(),
                "/TR".to_string(),
                task,
            ],
        )],
        uninstall_commands: vec![(
            "schtasks".to_string(),
            vec![
                "/Delete".to_string(),
                "/F".to_string(),
                "/TN".to_string(),
                JOB_ID.to_string(),
            ],
        )],
    })
}

fn windows_schedule(interval_minutes: u64) -> Result<(String, String)> {
    if interval_minutes < 24 * 60 {
        return Ok(("MINUTE".to_string(), interval_minutes.max(1).to_string()));
    }

    if interval_minutes.is_multiple_of(24 * 60) {
        return Ok((
            "DAILY".to_string(),
            (interval_minutes / (24 * 60)).max(1).to_string(),
        ));
    }

    bail!("Windows Task Scheduler supports autosubmit intervals under 24h or whole-day multiples")
}

pub fn replace_cron_block(existing: &str, block: &str) -> String {
    let mut output = Vec::new();
    let mut inside = false;
    for line in existing.lines() {
        if line.trim() == CRON_MARKER_BEGIN {
            inside = true;
            continue;
        }
        if line.trim() == CRON_MARKER_END {
            inside = false;
            continue;
        }
        if !inside {
            output.push(line.to_string());
        }
    }
    output.push(block.to_string());
    output.join("\n") + "\n"
}

fn install_cron_block(block: &str) -> Result<()> {
    let existing = read_crontab().unwrap_or_default();
    let updated = replace_cron_block(&existing, block);
    write_crontab(&updated)
}

fn uninstall_cron_block() -> Result<()> {
    let existing = read_crontab().unwrap_or_default();
    let updated = remove_cron_block(&existing);
    write_crontab(&updated)
}

fn remove_cron_block(existing: &str) -> String {
    let mut output = Vec::new();
    let mut inside = false;
    for line in existing.lines() {
        if line.trim() == CRON_MARKER_BEGIN {
            inside = true;
            continue;
        }
        if line.trim() == CRON_MARKER_END {
            inside = false;
            continue;
        }
        if !inside {
            output.push(line.to_string());
        }
    }
    if output.is_empty() {
        String::new()
    } else {
        output.join("\n") + "\n"
    }
}

fn read_crontab() -> Result<String> {
    let output = Command::new("crontab").arg("-l").output()?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Ok(String::new())
    }
}

fn write_crontab(content: &str) -> Result<()> {
    use std::io::Write;
    let mut child = Command::new("crontab")
        .arg("-")
        .stdin(std::process::Stdio::piped())
        .spawn()?;
    if let Some(stdin) = child.stdin.as_mut() {
        stdin.write_all(content.as_bytes())?;
    }
    let status = child.wait()?;
    if !status.success() {
        bail!("crontab exited with status {status}");
    }
    Ok(())
}

fn run_status_command(program: &str, args: &[String]) -> Result<()> {
    let status = Command::new(program).args(args).status()?;
    if !status.success() {
        bail!("{program} exited with status {status}");
    }
    Ok(())
}

fn autosubmit_log_path() -> Result<PathBuf> {
    let dir = crate::paths::get_config_dir().join("autosubmit");
    fs::create_dir_all(&dir)?;
    Ok(dir.join("autosubmit.log"))
}

fn autosubmit_lock_path() -> Result<PathBuf> {
    let dir = crate::paths::get_config_dir().join("autosubmit");
    fs::create_dir_all(&dir)?;
    Ok(dir.join("autosubmit.lock"))
}

fn validate_scheduler_executable(path: &Path) -> Result<()> {
    let rendered = path.to_string_lossy();
    if rendered.contains('\n') || rendered.contains('\r') || rendered.contains('\0') {
        bail!("Executable path contains unsupported control characters");
    }
    Ok(())
}

pub fn format_timestamp_ms(timestamp_ms: i64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(timestamp_ms)
        .map(|timestamp| timestamp.to_rfc3339())
        .unwrap_or_else(|| timestamp_ms.to_string())
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r#"'\''"#))
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn systemd_escape_path(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "\\\\")
        .replace(' ', "\\x20")
}

fn build_date_filter_for_date(
    date: &DateRangeFlags,
    current_date: chrono::NaiveDate,
) -> (Option<String>, Option<String>) {
    use chrono::{Datelike, Duration};

    if date.today {
        let day = current_date.format("%Y-%m-%d").to_string();
        return (Some(day.clone()), Some(day));
    }
    if date.yesterday {
        let day = (current_date - Duration::days(1))
            .format("%Y-%m-%d")
            .to_string();
        return (Some(day.clone()), Some(day));
    }
    if date.week {
        let start = current_date - Duration::days(6);
        return (
            Some(start.format("%Y-%m-%d").to_string()),
            Some(current_date.format("%Y-%m-%d").to_string()),
        );
    }
    if date.month {
        let start = current_date.with_day(1).unwrap_or(current_date);
        return (
            Some(start.format("%Y-%m-%d").to_string()),
            Some(current_date.format("%Y-%m-%d").to_string()),
        );
    }
    (date.since.clone(), date.until.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::ffi::{OsStr, OsString};
    use tempfile::TempDir;

    struct EnvVarGuard {
        key: &'static str,
        previous: Option<OsString>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: impl AsRef<OsStr>) -> Self {
            let previous = env::var_os(key);
            env::set_var(key, value);
            Self { key, previous }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            match self.previous.take() {
                Some(value) => env::set_var(self.key, value),
                None => env::remove_var(self.key),
            }
        }
    }

    #[test]
    fn parses_bounded_intervals() {
        assert_eq!(parse_interval_minutes("15m").unwrap(), 15);
        assert_eq!(parse_interval_minutes("2h").unwrap(), 120);
        assert_eq!(parse_interval_minutes("1d").unwrap(), 1440);
        assert!(parse_interval_minutes("14m").is_err());
        assert!(parse_interval_minutes("8d").is_err());
        assert!(parse_interval_minutes("1w").is_err());
    }

    #[test]
    fn run_decision_respects_interval_and_force() {
        let settings = AutosubmitSettings {
            enabled: true,
            interval_minutes: 60,
            last_run_at_ms: Some(1_000),
            ..AutosubmitSettings::default()
        };
        assert_eq!(
            run_decision(&settings, 30_000, false),
            AutosubmitRunDecision::NotDue {
                next_run_at_ms: 3_601_000
            }
        );
        assert_eq!(
            run_decision(&settings, 30_000, true),
            AutosubmitRunDecision::Due
        );
        assert_eq!(
            run_decision(&settings, 3_601_000, false),
            AutosubmitRunDecision::Due
        );
    }

    #[test]
    fn clients_for_settings_keep_empty_as_submit_default_marker() {
        let settings_clients = clients_for_settings(ClientFlags::default());
        assert!(settings_clients.is_empty());
    }

    #[test]
    fn cron_block_replacement_preserves_unrelated_jobs() {
        let existing =
            "0 0 * * * echo keep\n# BEGIN TOKSCALE AUTOSUBMIT\nold\n# END TOKSCALE AUTOSUBMIT\n";
        let updated = replace_cron_block(
            existing,
            "# BEGIN TOKSCALE AUTOSUBMIT\nnew\n# END TOKSCALE AUTOSUBMIT",
        );
        assert!(updated.contains("0 0 * * * echo keep"));
        assert!(updated.contains("new"));
        assert!(!updated.contains("old"));
    }

    #[test]
    fn launchd_spec_uses_program_arguments_without_shell() {
        let settings = AutosubmitSettings {
            interval_minutes: 60,
            ..AutosubmitSettings::default()
        };
        let spec = render_launchd_spec(Path::new("/usr/local/bin/tokscale"), &settings).unwrap();
        let content = &spec.files[0].1;
        assert!(content.contains("<string>/usr/local/bin/tokscale</string>"));
        assert!(content.contains("<string>autosubmit</string>"));
        assert!(content.contains("<string>run</string>"));
        assert!(!content.contains("/bin/sh"));
    }

    #[test]
    fn systemd_spec_uses_autosubmit_run() {
        let settings = AutosubmitSettings {
            interval_minutes: 120,
            ..AutosubmitSettings::default()
        };
        let spec = render_systemd_spec(Path::new("/usr/local/bin/tokscale"), &settings).unwrap();
        let service = &spec.files[0].1;
        let timer = &spec.files[1].1;
        assert!(service.contains("ExecStart=/usr/local/bin/tokscale autosubmit run"));
        assert!(timer.contains("OnUnitActiveSec=120min"));
    }

    #[test]
    #[serial_test::serial]
    fn systemd_spec_honors_xdg_config_home() {
        let temp = TempDir::new().unwrap();
        let _guard = EnvVarGuard::set("XDG_CONFIG_HOME", temp.path());
        let settings = AutosubmitSettings::default();

        let spec = render_systemd_spec(Path::new("/usr/local/bin/tokscale"), &settings).unwrap();

        assert_eq!(
            spec.files[0].0,
            temp.path()
                .join("systemd")
                .join("user")
                .join("tokscale-autosubmit.service")
        );
        assert_eq!(
            spec.files[1].0,
            temp.path()
                .join("systemd")
                .join("user")
                .join("tokscale-autosubmit.timer")
        );
    }

    #[test]
    fn windows_spec_uses_fixed_task_name() {
        let settings = AutosubmitSettings {
            interval_minutes: 30,
            ..AutosubmitSettings::default()
        };
        let spec = render_windows_task_spec(Path::new("C:/bin/tokscale.exe"), &settings).unwrap();
        let args = &spec.install_commands[0].1;
        assert!(args.iter().any(|arg| arg == JOB_ID));
        assert!(args.iter().any(|arg| arg.contains("autosubmit run")));
        assert!(args
            .windows(2)
            .any(|pair| pair[0] == "/SC" && pair[1] == "MINUTE"));
        assert!(args
            .windows(2)
            .any(|pair| pair[0] == "/MO" && pair[1] == "30"));
    }

    #[test]
    fn windows_spec_uses_daily_schedule_for_default_interval() {
        let spec = render_windows_task_spec(
            Path::new("C:/bin/tokscale.exe"),
            &AutosubmitSettings::default(),
        )
        .unwrap();
        let args = &spec.install_commands[0].1;
        assert!(args
            .windows(2)
            .any(|pair| pair[0] == "/SC" && pair[1] == "DAILY"));
        assert!(args
            .windows(2)
            .any(|pair| pair[0] == "/MO" && pair[1] == "1"));
    }

    #[test]
    fn windows_spec_rejects_long_non_day_interval() {
        let settings = AutosubmitSettings {
            interval_minutes: 25 * 60,
            ..AutosubmitSettings::default()
        };

        let err = render_windows_task_spec(Path::new("C:/bin/tokscale.exe"), &settings)
            .expect_err("25h is not representable by schtasks minute or daily cadence");
        assert!(err.to_string().contains("whole-day multiples"));
    }

    #[test]
    fn submit_filters_keep_absolute_date_filters() {
        let settings = AutosubmitSettings {
            clients: vec!["opencode".to_string(), "claude".to_string()],
            since: Some("2026-01-01".to_string()),
            until: Some("2026-01-31".to_string()),
            ..AutosubmitSettings::default()
        };

        let (clients, since, until, year) = submit_filters(&settings);

        assert_eq!(
            clients,
            Some(vec!["opencode".to_string(), "claude".to_string()])
        );
        assert_eq!(since.as_deref(), Some("2026-01-01"));
        assert_eq!(until.as_deref(), Some("2026-01-31"));
        assert_eq!(year, None);
    }

    #[test]
    fn submit_filters_default_to_submit_clients_when_unfiltered() {
        let settings = AutosubmitSettings::default();

        let (clients, _, _, _) = submit_filters(&settings);

        let clients = clients.unwrap();
        assert!(clients.contains(&"opencode".to_string()));
        assert!(clients.contains(&"synthetic".to_string()));
        assert!(!clients.contains(&"warp".to_string()));
    }

    #[test]
    #[serial_test::serial]
    fn run_lock_blocks_concurrent_holder() {
        let temp = TempDir::new().unwrap();
        let _guard = EnvVarGuard::set("TOKSCALE_CONFIG_DIR", temp.path());

        let first = try_acquire_run_lock().unwrap().expect("first lock");
        assert!(try_acquire_run_lock().unwrap().is_none());
        drop(first);
        assert!(try_acquire_run_lock().unwrap().is_some());
    }
}
