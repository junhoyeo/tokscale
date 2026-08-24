use crate::tui::settings::{
    AutosubmitSettings, MAX_AUTOSUBMIT_INTERVAL_MINUTES, MIN_AUTOSUBMIT_INTERVAL_MINUTES,
};
use crate::{build_date_filter_for_date, normalize_year_filter, ClientFlags, DateRangeFlags};
use anyhow::{anyhow, bail, Context, Result};
use clap::{Args, Subcommand, ValueEnum};
use fs2::FileExt;
use serde::Serialize;
use std::fs::{self, OpenOptions};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::Command;
#[cfg(target_os = "windows")]
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const JOB_ID: &str = "ai.tokscale.autosubmit";
/// Version of the build answering the current call, stamped into settings when
/// the managed copy is written so later runs can tell whether it has drifted.
const RUNNING_VERSION: &str = env!("CARGO_PKG_VERSION");
const CRON_MARKER_BEGIN: &str = "# BEGIN TOKSCALE AUTOSUBMIT";
const CRON_MARKER_END: &str = "# END TOKSCALE AUTOSUBMIT";
const SKIP_SCHEDULER_ENV: &str = "TOKSCALE_AUTOSUBMIT_SKIP_SCHEDULER";
const SYSTEMD_TIMER_UNIT: &str = "tokscale-autosubmit.timer";
const MANAGED_EXECUTABLE_NAME: &str = if cfg!(target_os = "windows") {
    "tokscale.exe"
} else {
    "tokscale"
};
#[cfg(any(target_os = "windows", test))]
const LEGACY_WINDOWS_MANAGED_EXECUTABLE_NAME: &str = "tokscale.exe";
#[cfg(target_os = "windows")]
static WINDOWS_MANAGED_EXECUTABLE_SEQUENCE: AtomicU64 = AtomicU64::new(0);
#[cfg(target_os = "windows")]
const WINDOWS_ERROR_SHARING_VIOLATION: i32 = 32;

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

#[derive(Debug, Clone, PartialEq, Eq)]
struct LaunchdService {
    domain: String,
    target: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CapturedCommand {
    command: String,
    status: String,
    success: bool,
    stdout: String,
    stderr: String,
}

#[derive(Debug, Clone)]
struct SchedulerArtifactSnapshot {
    scheduler: SchedulerKind,
    files: Vec<(PathBuf, Option<Vec<u8>>)>,
    cron: Option<String>,
    executable: PathBuf,
}

#[derive(Debug, Clone)]
struct ManagedExecutableSnapshot {
    path: PathBuf,
    contents: Option<Vec<u8>>,
    permissions: Option<fs::Permissions>,
}

struct EnableRollbackContext<'a> {
    previous_settings: &'a crate::tui::settings::Settings,
    previous_artifacts: Option<&'a SchedulerArtifactSnapshot>,
    managed_snapshot: &'a ManagedExecutableSnapshot,
    scheduler: SchedulerKind,
    scheduler_started: bool,
    previous_scheduler_was_displaced: bool,
    exe: &'a Path,
    next_settings: &'a AutosubmitSettings,
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
    managed_executable: Option<String>,
    managed_executable_version: Option<String>,
    /// True when the scheduled copy came from a different build than the one
    /// answering this call. Scripted health checks read this rather than
    /// diffing the two version strings themselves.
    managed_executable_stale: bool,
    last_run_at_ms: Option<i64>,
    last_error: Option<String>,
}

pub fn enable(args: AutosubmitEnableArgs) -> Result<()> {
    enable_with_scheduler_operations(args, install_scheduler, |scheduler, _, settings| {
        uninstall_scheduler(scheduler, settings)
    })
}

fn enable_with_scheduler_operations<I, U>(
    args: AutosubmitEnableArgs,
    mut installer: I,
    mut uninstaller: U,
) -> Result<()>
where
    I: FnMut(SchedulerKind, &Path, &AutosubmitSettings) -> Result<()>,
    U: FnMut(SchedulerKind, &Path, &AutosubmitSettings) -> Result<()>,
{
    let interval_minutes = parse_interval_minutes(&args.interval)?;
    let scheduler = args.scheduler.unwrap_or_else(default_scheduler_kind);
    let previous_settings = crate::tui::settings::Settings::load();
    let previous_scheduler = previous_settings
        .autosubmit
        .scheduler
        .as_deref()
        .and_then(SchedulerKind::from_str)
        .unwrap_or_else(default_scheduler_kind);
    let next_autosubmit = AutosubmitSettings {
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
        last_run_at_ms: previous_settings.autosubmit.last_run_at_ms,
        last_error: None,
        managed_executable: None,
        managed_executable_version: None,
    };
    let source_exe =
        std::env::current_exe().context("Could not resolve current tokscale executable")?;
    let managed_destination = next_managed_scheduler_executable_path()?;
    let managed_snapshot = snapshot_managed_scheduler_executable(&managed_destination)?;
    let previous_artifacts = if previous_settings.autosubmit.enabled && !skip_scheduler_install() {
        Some(snapshot_scheduler_artifacts(
            previous_scheduler,
            &previous_settings.autosubmit,
        )?)
    } else {
        None
    };
    let exe = match prepare_managed_scheduler_executable(&source_exe, managed_destination) {
        Ok(exe) => exe,
        Err(error) => match restore_managed_scheduler_executable(&managed_snapshot) {
            Ok(()) => return Err(error),
            Err(rollback_error) => {
                return Err(anyhow!(
                    "{error}; rollback failed while restoring managed executable: {rollback_error}"
                ));
            }
        },
    };
    let mut next_autosubmit = next_autosubmit;
    next_autosubmit.managed_executable = Some(exe.to_string_lossy().into_owned());
    // The version is deliberately NOT stamped here. This save happens before the
    // scheduler is installed, and on Windows `exe` is a freshly versioned path
    // while the task still points at the previous one. Recording the version now
    // would mean that a re-enable killed between this save and the install below
    // leaves settings claiming a build the scheduler is not running, and
    // `managed_executable_is_stale` would report clean at exactly the moment it
    // is wrong. It is stamped after installation succeeds instead; until then
    // `None` reads as "unknown", which reports drift.

    let mut next_settings = previous_settings.clone();
    next_settings.autosubmit = next_autosubmit;
    if let Err(error) = next_settings.save() {
        return enable_failure_with_rollback(
            error,
            EnableRollbackContext {
                previous_settings: &previous_settings,
                previous_artifacts: previous_artifacts.as_ref(),
                managed_snapshot: &managed_snapshot,
                scheduler,
                scheduler_started: false,
                previous_scheduler_was_displaced: false,
                exe: &exe,
                next_settings: &next_settings.autosubmit,
            },
            &mut uninstaller,
        );
    }

    if !skip_scheduler_install() {
        let retargets_previous_windows_task = previous_settings.autosubmit.enabled
            && previous_scheduler == SchedulerKind::WindowsTaskScheduler
            && scheduler == SchedulerKind::WindowsTaskScheduler;
        let mut previous_scheduler_was_displaced = false;
        if previous_settings.autosubmit.enabled && !retargets_previous_windows_task {
            previous_scheduler_was_displaced = true;
            if let Err(error) = uninstaller(previous_scheduler, &exe, &previous_settings.autosubmit)
            {
                return enable_failure_with_rollback(
                    error,
                    EnableRollbackContext {
                        previous_settings: &previous_settings,
                        previous_artifacts: previous_artifacts.as_ref(),
                        managed_snapshot: &managed_snapshot,
                        scheduler,
                        scheduler_started: false,
                        previous_scheduler_was_displaced,
                        exe: &exe,
                        next_settings: &next_settings.autosubmit,
                    },
                    &mut uninstaller,
                );
            }
        }

        if let Err(error) = installer(scheduler, &exe, &next_settings.autosubmit) {
            return enable_failure_with_rollback(
                error,
                EnableRollbackContext {
                    previous_settings: &previous_settings,
                    previous_artifacts: previous_artifacts.as_ref(),
                    managed_snapshot: &managed_snapshot,
                    scheduler,
                    scheduler_started: !retargets_previous_windows_task,
                    previous_scheduler_was_displaced,
                    exe: &exe,
                    next_settings: &next_settings.autosubmit,
                },
                &mut uninstaller,
            );
        }
    }

    // Second phase of the version stamp: only now is the scheduler actually
    // pointing at `exe`, so only now does recording its build describe reality.
    //
    // A failure here is not worth unwinding a correctly installed scheduler. The
    // version stays `None`, status reports drift, and re-running `enable` clears
    // it — the conservative direction, and self-healing.
    next_settings.autosubmit.managed_executable_version = Some(RUNNING_VERSION.to_string());
    if let Err(error) = next_settings.save() {
        eprintln!(
            "Warning: autosubmit is enabled, but the scheduled build could not be recorded: {error}\n         \
             `tokscale autosubmit status` will report it as out of date until you re-run `enable`."
        );
    }

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
        if managed_executable_is_stale(&autosubmit) {
            let scheduled = autosubmit
                .managed_executable_version
                .as_deref()
                .unwrap_or("unknown");
            println!(
                "  Scheduled binary: {scheduled} (this build is {RUNNING_VERSION})\n    \
                 The scheduler runs its own copy, which upgrades do not replace. \
                 Refresh it with:\n      {}",
                enable_command_for(&autosubmit)
            );
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
    disable_with_scheduler_operations(uninstall_scheduler, remove_managed_scheduler_executable)
}

fn disable_with_scheduler_operations<U, R>(
    mut uninstaller: U,
    mut executable_cleanup: R,
) -> Result<()>
where
    U: FnMut(SchedulerKind, &AutosubmitSettings) -> Result<()>,
    R: FnMut() -> Result<()>,
{
    let mut settings = crate::tui::settings::Settings::load();
    let scheduler = settings
        .autosubmit
        .scheduler
        .as_deref()
        .and_then(SchedulerKind::from_str)
        .unwrap_or_else(default_scheduler_kind);

    if !skip_scheduler_install() {
        if settings.autosubmit.enabled {
            uninstaller(scheduler, &settings.autosubmit)?;
        }
        executable_cleanup()?;
    }

    settings.autosubmit.enabled = false;
    settings.autosubmit.managed_executable = None;
    settings.autosubmit.managed_executable_version = None;
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

/// Today, in the zone this device buckets day keys into.
///
/// A scheduled run submits, so "today" has to be the pinned zone's today — the
/// same one the day buckets are keyed by. Reading the host clock here would
/// select a partial pinned day out of correctly-keyed buckets, and a partial
/// day is exactly what the server's monotonic per-day guard then freezes at the
/// low value. `--home` is rejected for autosubmit, so there is only ever one
/// profile in play.
fn current_bucket_date() -> chrono::NaiveDate {
    tokscale_core::BucketTimezone::from_scanner_settings(
        &crate::tui::settings::load_scanner_settings(),
    )
    .today()
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
    let (since, until) = build_date_filter_for_date(&date, current_bucket_date());
    let year = normalize_year_filter(&date);
    (clients, since, until, year)
}

/// A contended `LockFileEx` on Windows fails with `ERROR_LOCK_VIOLATION`,
/// which `std` leaves uncategorised instead of mapping to `WouldBlock`.
///
/// Deliberately not behind `cfg(windows)`: the bug this guards against only
/// reproduces on a platform CI does not run, so keeping the constant
/// unconditional is what lets the classification be unit-tested at all. No
/// Unix `flock` failure uses errno 33, so matching it everywhere cannot
/// disguise a genuine error as contention.
const WINDOWS_ERROR_LOCK_VIOLATION: i32 = 33;

/// Whether `err` means "another process already holds this lock" rather than a
/// real I/O failure.
///
/// Only `WouldBlock` used to count, so on Windows a second scheduled run hit
/// the error arm and the caller's `unwrap` panicked — a run that found another
/// already in progress failed instead of stepping aside.
pub(crate) fn is_lock_contention(err: &std::io::Error) -> bool {
    err.kind() == ErrorKind::WouldBlock || err.raw_os_error() == Some(WINDOWS_ERROR_LOCK_VIOLATION)
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
        Err(err) if is_lock_contention(&err) => Ok(None),
        Err(err) => Err(err)
            .with_context(|| format!("Could not lock autosubmit state at {}", path.display())),
    }
}

/// Whether the scheduled copy came from a different build than the one running
/// now, meaning the scheduler is submitting with stale code.
///
/// `enable` is the only writer of the managed copy, so replacing the installed
/// binary — `npm update -g tokscale`, brew, or dropping in a new binary — does
/// not touch it and the scheduled job silently keeps the old build.
///
/// Compared by version rather than by content. Hashing would additionally catch
/// same-version rebuilds, which only developers produce, at the cost of reading
/// the whole binary on every status call.
///
/// Any difference counts, not just an older recorded version: a deliberate
/// downgrade is drift too. The question being answered is "what is the
/// scheduler actually running", not "which build is newer".
fn managed_executable_is_stale(settings: &AutosubmitSettings) -> bool {
    if !settings.enabled {
        return false;
    }
    let Some(managed) = settings.managed_executable.as_deref() else {
        return false;
    };
    // Invoking the managed copy directly compares it against itself, which
    // always matches and says nothing. Report no drift rather than a spurious
    // clean bill of health from a binary that cannot observe its own staleness.
    if running_executable_is(managed) {
        return false;
    }
    // `None` predates this field, so the recorded build is genuinely unknown
    // and reported as drift. It resolves on the next enable.
    settings.managed_executable_version.as_deref() != Some(RUNNING_VERSION)
}

/// The `enable` invocation that reproduces the configuration already in
/// settings.
///
/// `enable` rebuilds every field from its arguments — only `last_run_at_ms`
/// survives — and `--interval` defaults to `24h`. So telling somebody to
/// "re-run `tokscale autosubmit enable`" after an upgrade would silently reset
/// a 30m interval to 24h and drop any client or date filter. Printing what they
/// actually configured makes the advice safe to follow verbatim.
fn enable_command_for(settings: &AutosubmitSettings) -> String {
    let mut parts = vec![
        "tokscale autosubmit enable".to_string(),
        format!("--interval {}m", settings.interval_minutes),
    ];
    if !settings.clients.is_empty() {
        parts.push(format!("--client {}", settings.clients.join(",")));
    }
    if let Some(since) = settings.since.as_deref() {
        parts.push(format!("--since {since}"));
    }
    if let Some(until) = settings.until.as_deref() {
        parts.push(format!("--until {until}"));
    }
    if let Some(year) = settings.year.as_deref() {
        parts.push(format!("--year {year}"));
    }
    for (flag, enabled) in [
        ("--today", settings.today),
        ("--yesterday", settings.yesterday),
        ("--week", settings.week),
        ("--month", settings.month),
    ] {
        if enabled {
            parts.push(flag.to_string());
        }
    }
    if let Some(scheduler) = settings.scheduler.as_deref() {
        parts.push(format!("--scheduler {scheduler}"));
    }
    parts.join(" ")
}

/// Whether the process answering this call is the binary at `path`, compared
/// through the filesystem so a symlinked or relative path still matches.
fn running_executable_is(path: &str) -> bool {
    let Ok(current) = std::env::current_exe() else {
        return false;
    };
    let candidate = Path::new(path);
    match (current.canonicalize(), candidate.canonicalize()) {
        (Ok(current), Ok(candidate)) => current == candidate,
        // A managed copy that no longer exists cannot be the running process,
        // and is a separate problem from drift.
        _ => current == candidate,
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
        managed_executable: settings.managed_executable.clone(),
        managed_executable_version: settings.managed_executable_version.clone(),
        managed_executable_stale: managed_executable_is_stale(settings),
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
    for (path, content) in &spec.files {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, content)?;
    }
    if scheduler == SchedulerKind::Launchd {
        let plist = spec
            .files
            .first()
            .map(|(path, _)| path)
            .context("launchd scheduler is missing its plist")?;
        activate_launchd_service(plist)?;
    } else {
        for (program, args) in spec.install_commands {
            run_status_command(&program, &args)?;
        }
    }
    if let Some(block) = spec.cron_block {
        install_cron_block(&block)?;
    }
    Ok(())
}

fn uninstall_scheduler(scheduler: SchedulerKind, settings: &AutosubmitSettings) -> Result<()> {
    let exe = managed_scheduler_executable_for_settings(settings)?;
    let spec = render_scheduler_spec(scheduler, &exe, settings)?;
    if scheduler == SchedulerKind::Launchd {
        deactivate_launchd_service()?;
    } else {
        for (program, args) in spec.uninstall_commands {
            run_scheduler_cleanup_command(scheduler, &program, &args)?;
        }
    }
    if scheduler == SchedulerKind::Cron {
        uninstall_cron_block()?;
    }
    for (path, _) in spec.files {
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(err) if err.kind() == ErrorKind::NotFound => {}
            Err(err) => return Err(err.into()),
        }
    }
    Ok(())
}

fn snapshot_managed_scheduler_executable(path: &Path) -> Result<ManagedExecutableSnapshot> {
    let path = path.to_path_buf();
    match fs::read(&path) {
        Ok(contents) => Ok(ManagedExecutableSnapshot {
            permissions: Some(fs::metadata(&path)?.permissions()),
            path,
            contents: Some(contents),
        }),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(ManagedExecutableSnapshot {
            path,
            contents: None,
            permissions: None,
        }),
        Err(error) => Err(error.into()),
    }
}

fn restore_managed_scheduler_executable(snapshot: &ManagedExecutableSnapshot) -> Result<()> {
    let Some(contents) = &snapshot.contents else {
        return match fs::remove_file(&snapshot.path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        };
    };
    let permissions = snapshot
        .permissions
        .as_ref()
        .context("Managed executable snapshot is missing permissions")?;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let temporary = snapshot.path.with_file_name(format!(
        ".{MANAGED_EXECUTABLE_NAME}.restore.{}.{}.tmp",
        std::process::id(),
        timestamp
    ));

    let result = (|| -> Result<()> {
        fs::write(&temporary, contents)?;
        fs::set_permissions(&temporary, permissions.clone())?;
        OpenOptions::new()
            .write(true)
            .open(&temporary)?
            .sync_all()?;
        tokscale_core::fs_atomic::replace_file(&temporary, &snapshot.path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result?;
    validate_managed_scheduler_executable(&snapshot.path)
}

fn snapshot_scheduler_artifacts(
    scheduler: SchedulerKind,
    settings: &AutosubmitSettings,
) -> Result<SchedulerArtifactSnapshot> {
    let exe = managed_scheduler_executable_for_settings(settings)?;
    let spec = render_scheduler_spec(scheduler, &exe, settings)?;
    let mut files = Vec::with_capacity(spec.files.len());
    for (path, _) in spec.files {
        let contents = match fs::read(&path) {
            Ok(contents) => Some(contents),
            Err(error) if error.kind() == ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        files.push((path, contents));
    }
    let cron = spec.cron_block.map(|_| read_crontab()).transpose()?;
    Ok(SchedulerArtifactSnapshot {
        executable: exe,
        scheduler,
        files,
        cron,
    })
}

fn restore_scheduler_artifacts(snapshot: &SchedulerArtifactSnapshot) -> Result<()> {
    for (path, contents) in &snapshot.files {
        match contents {
            Some(contents) => {
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(path, contents)?;
            }
            None => match fs::remove_file(path) {
                Ok(()) => {}
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            },
        }
    }
    if let Some(cron) = &snapshot.cron {
        write_crontab(cron)?;
    }
    Ok(())
}

fn reactivate_scheduler(
    scheduler: SchedulerKind,
    exe: &Path,
    settings: &AutosubmitSettings,
) -> Result<()> {
    let spec = render_scheduler_spec(scheduler, exe, settings)?;
    if scheduler == SchedulerKind::Launchd {
        let plist = spec
            .files
            .first()
            .map(|(path, _)| path)
            .context("launchd scheduler is missing its plist")?;
        let service = active_launchd_service()?;
        let (program, args) = launchd_print_command(&service);
        let current = capture_command(&program, &args)?;
        if verify_launchd_service(&service, &current).is_err() {
            activate_launchd_service(plist)?;
        }
        return Ok(());
    }
    for (program, args) in spec.install_commands {
        run_status_command(&program, &args)?;
    }
    Ok(())
}

fn enable_failure_with_rollback<U>(
    error: anyhow::Error,
    context: EnableRollbackContext<'_>,
    uninstaller: &mut U,
) -> Result<()>
where
    U: FnMut(SchedulerKind, &Path, &AutosubmitSettings) -> Result<()>,
{
    let mut rollback_errors = Vec::new();
    if context.scheduler_started {
        if let Err(rollback_error) =
            uninstaller(context.scheduler, context.exe, context.next_settings)
        {
            rollback_errors.push(format!("removing new scheduler: {rollback_error}"));
        }
    }
    if let Err(rollback_error) = context.previous_settings.save() {
        rollback_errors.push(format!("restoring settings: {rollback_error}"));
    }
    if let Err(rollback_error) = restore_managed_scheduler_executable(context.managed_snapshot) {
        rollback_errors.push(format!("restoring managed executable: {rollback_error}"));
    }
    if let Some(snapshot) = context.previous_artifacts {
        if let Err(rollback_error) = restore_scheduler_artifacts(snapshot) {
            rollback_errors.push(format!("restoring scheduler artifacts: {rollback_error}"));
        }
        if context.previous_scheduler_was_displaced {
            if let Err(rollback_error) = reactivate_scheduler(
                snapshot.scheduler,
                &snapshot.executable,
                &context.previous_settings.autosubmit,
            ) {
                rollback_errors.push(format!("reactivating previous scheduler: {rollback_error}"));
            }
        }
    }

    if rollback_errors.is_empty() {
        Err(error)
    } else {
        Err(anyhow!(
            "{error}; rollback failed: {}",
            rollback_errors.join("; ")
        ))
    }
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
    let home = crate::paths::home_dir().context("Could not determine home directory")?;
    render_launchd_spec_for_home(exe, settings, &home)
}

fn render_launchd_spec_for_home(
    exe: &Path,
    settings: &AutosubmitSettings,
    home: &Path,
) -> Result<SchedulerSpec> {
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
        files: vec![(plist_path, content)],
        cron_block: None,
        install_commands: Vec::new(),
        uninstall_commands: Vec::new(),
    })
}

fn render_systemd_spec(exe: &Path, settings: &AutosubmitSettings) -> Result<SchedulerSpec> {
    let user_dir = systemd_user_dir()?;
    let service_path = user_dir.join("tokscale-autosubmit.service");
    let timer_path = user_dir.join(SYSTEMD_TIMER_UNIT);
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
                    SYSTEMD_TIMER_UNIT.to_string(),
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
                    SYSTEMD_TIMER_UNIT.to_string(),
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
        .or_else(|| crate::paths::home_dir().map(|home| home.join(".config")));
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

fn run_scheduler_cleanup_command(
    scheduler: SchedulerKind,
    program: &str,
    args: &[String],
) -> Result<()> {
    let captured = capture_command(program, args)?;
    cleanup_scheduler_command_result(scheduler, "scheduler cleanup", &captured)
}

fn cleanup_scheduler_command_result(
    scheduler: SchedulerKind,
    action: &str,
    captured: &CapturedCommand,
) -> Result<()> {
    if captured.success || scheduler_entry_is_absent(scheduler, captured) {
        return Ok(());
    }
    Err(command_failure(action, captured))
}

fn scheduler_entry_is_absent(scheduler: SchedulerKind, captured: &CapturedCommand) -> bool {
    if captured.success {
        return false;
    }

    let diagnostic = format!("{}\n{}", captured.stdout, captured.stderr).to_lowercase();
    let systemd_timer = SYSTEMD_TIMER_UNIT.to_ascii_lowercase();
    let windows_task = JOB_ID.to_ascii_lowercase();
    match scheduler {
        SchedulerKind::Systemd => {
            diagnostic.contains(&format!("unit {systemd_timer} not loaded"))
                || diagnostic.contains(&format!("unit file {systemd_timer} does not exist"))
        }
        SchedulerKind::WindowsTaskScheduler => {
            diagnostic.contains("error: the system cannot find the file specified.")
                || diagnostic.contains(&format!(
                "error: the specified task name \"{windows_task}\" does not exist in the system."
            ))
        }
        SchedulerKind::Launchd | SchedulerKind::Cron => false,
    }
}

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn geteuid() -> u32;
}

#[cfg(any(target_os = "macos", test))]
fn launchd_service_for_uid(uid: u32) -> LaunchdService {
    let domain = format!("gui/{uid}");
    let target = format!("{domain}/{JOB_ID}");
    LaunchdService { domain, target }
}

#[cfg(target_os = "macos")]
fn active_launchd_service() -> Result<LaunchdService> {
    Ok(launchd_service_for_uid(unsafe { geteuid() }))
}

#[cfg(not(target_os = "macos"))]
fn active_launchd_service() -> Result<LaunchdService> {
    bail!("launchd scheduler is only available on macOS")
}

fn launchd_bootstrap_command(service: &LaunchdService, plist: &Path) -> (String, Vec<String>) {
    (
        "launchctl".to_string(),
        vec![
            "bootstrap".to_string(),
            service.domain.clone(),
            plist.to_string_lossy().into_owned(),
        ],
    )
}

fn launchd_bootout_command(service: &LaunchdService) -> (String, Vec<String>) {
    (
        "launchctl".to_string(),
        vec![
            "bootout".to_string(),
            "--wait".to_string(),
            service.target.clone(),
        ],
    )
}

fn launchd_print_command(service: &LaunchdService) -> (String, Vec<String>) {
    (
        "launchctl".to_string(),
        vec!["print".to_string(), service.target.clone()],
    )
}

fn activate_launchd_service(plist: &Path) -> Result<()> {
    let service = active_launchd_service()?;
    let (program, args) = launchd_bootstrap_command(&service, plist);
    let bootstrap = capture_command(&program, &args)?;
    if !bootstrap.success {
        return Err(command_failure("launchd bootstrap", &bootstrap));
    }

    let (program, args) = launchd_print_command(&service);
    let verification = capture_command(&program, &args)?;
    verify_launchd_service(&service, &verification)
}

fn deactivate_launchd_service() -> Result<()> {
    let service = active_launchd_service()?;
    let (program, args) = launchd_bootout_command(&service);
    let bootout = capture_command(&program, &args)?;
    if bootout.success {
        return Ok(());
    }

    let (program, args) = launchd_print_command(&service);
    let verification = capture_command(&program, &args)?;
    if launchd_service_is_absent(&service, &verification) {
        return Ok(());
    }

    Err(command_failure("launchd bootout", &bootout))
}

fn verify_launchd_service(service: &LaunchdService, captured: &CapturedCommand) -> Result<()> {
    if !captured.success || !captured.stdout.contains(&service.target) {
        return Err(command_failure("launchd print verification", captured));
    }
    Ok(())
}

fn launchd_service_is_absent(service: &LaunchdService, captured: &CapturedCommand) -> bool {
    if captured.success {
        return false;
    }
    let missing_service = format!("could not find service \"{}\"", service.target).to_lowercase();
    let diagnostic = format!("{}\n{}", captured.stdout, captured.stderr).to_lowercase();
    diagnostic.contains(&missing_service)
}

fn capture_command(program: &str, args: &[String]) -> Result<CapturedCommand> {
    let command = command_display(program, args);
    let output = Command::new(program)
        .args(args)
        .output()
        .with_context(|| format!("Could not execute `{command}`"))?;
    Ok(CapturedCommand {
        command,
        status: output.status.to_string(),
        success: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

fn command_display(program: &str, args: &[String]) -> String {
    std::iter::once(program)
        .chain(args.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join(" ")
}

fn command_failure(action: &str, captured: &CapturedCommand) -> anyhow::Error {
    anyhow!(
        "{action} failed: command `{}` exited with status {}; stdout: {}; stderr: {}",
        captured.command,
        captured.status,
        captured.stdout.trim(),
        captured.stderr.trim()
    )
}

fn autosubmit_dir() -> Result<PathBuf> {
    let dir = crate::paths::get_config_dir().join("autosubmit");
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn autosubmit_log_path() -> Result<PathBuf> {
    Ok(autosubmit_dir()?.join("autosubmit.log"))
}

fn autosubmit_lock_path() -> Result<PathBuf> {
    Ok(autosubmit_dir()?.join("autosubmit.lock"))
}

fn managed_scheduler_executable_path() -> Result<PathBuf> {
    Ok(autosubmit_dir()?.join(MANAGED_EXECUTABLE_NAME))
}

fn managed_scheduler_executable_for_settings(settings: &AutosubmitSettings) -> Result<PathBuf> {
    let path = settings
        .managed_executable
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or(managed_scheduler_executable_path()?);
    validate_scheduler_executable(&path)?;
    Ok(path)
}

fn next_managed_scheduler_executable_path() -> Result<PathBuf> {
    let dir = autosubmit_dir()?;
    #[cfg(target_os = "windows")]
    {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let sequence = WINDOWS_MANAGED_EXECUTABLE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        return Ok(versioned_windows_managed_executable_path(
            &dir, timestamp, sequence,
        ));
    }
    #[cfg(not(target_os = "windows"))]
    {
        Ok(dir.join(MANAGED_EXECUTABLE_NAME))
    }
}

#[cfg(any(target_os = "windows", test))]
fn versioned_windows_managed_executable_path(
    autosubmit_dir: &Path,
    timestamp: u128,
    sequence: u64,
) -> PathBuf {
    autosubmit_dir.join(format!(
        "tokscale-{}-{timestamp}-{sequence}.exe",
        std::process::id()
    ))
}

fn prepare_managed_scheduler_executable(source: &Path, managed: PathBuf) -> Result<PathBuf> {
    validate_scheduler_executable(source)?;
    let source_metadata = fs::metadata(source)
        .with_context(|| format!("Could not read tokscale executable at {}", source.display()))?;
    if !source_metadata.is_file() {
        bail!(
            "Tokscale executable is not a regular file: {}",
            source.display()
        );
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if source_metadata.permissions().mode() & 0o111 == 0 {
            bail!(
                "Tokscale executable is not executable: {}",
                source.display()
            );
        }
    }

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let temporary = managed.with_file_name(format!(
        ".{MANAGED_EXECUTABLE_NAME}.{}.{}.tmp",
        std::process::id(),
        timestamp
    ));

    let result = (|| -> Result<()> {
        fs::copy(source, &temporary).with_context(|| {
            format!(
                "Could not copy tokscale executable from {} to {}",
                source.display(),
                temporary.display()
            )
        })?;
        fs::set_permissions(&temporary, source_metadata.permissions())?;
        OpenOptions::new()
            .write(true)
            .open(&temporary)?
            .sync_all()?;
        tokscale_core::fs_atomic::replace_file(&temporary, &managed)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result?;

    validate_managed_scheduler_executable(&managed)?;
    Ok(managed)
}

#[cfg(any(target_os = "windows", test))]
fn is_windows_managed_executable_name(name: &str) -> bool {
    name == LEGACY_WINDOWS_MANAGED_EXECUTABLE_NAME
        || (name.starts_with("tokscale-") && name.ends_with(".exe"))
}

fn remove_managed_scheduler_executable() -> Result<()> {
    #[cfg(target_os = "windows")]
    {
        for entry in fs::read_dir(autosubmit_dir()?)? {
            let path = entry?.path();
            let is_managed_executable = path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(is_windows_managed_executable_name);
            if !is_managed_executable {
                continue;
            }
            match fs::remove_file(path) {
                Ok(()) => {}
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                Err(error) if error.raw_os_error() == Some(WINDOWS_ERROR_SHARING_VIOLATION) => {}
                Err(error) => return Err(error.into()),
            }
        }
        return Ok(());
    }
    #[cfg(not(target_os = "windows"))]
    {
        let managed = managed_scheduler_executable_path()?;
        match fs::remove_file(managed) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
}

fn validate_managed_scheduler_executable(path: &Path) -> Result<()> {
    validate_scheduler_executable(path)?;
    let metadata = fs::metadata(path)
        .with_context(|| format!("Managed tokscale executable is missing: {}", path.display()))?;
    if !metadata.is_file() {
        bail!(
            "Managed tokscale executable is not a regular file: {}",
            path.display()
        );
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            bail!(
                "Managed tokscale executable is not executable: {}",
                path.display()
            );
        }
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::ffi::{OsStr, OsString};
    use tempfile::TempDir;

    /// `TOKSCALE_CONFIG_DIR` is process-global while cargo runs tests on
    /// parallel threads, so every test that redirects it needs
    /// `#[serial_test::serial]` — the spelling already used throughout this
    /// module. Without it they read each other's config directory: one saves
    /// settings, another's guard restores the variable underneath it, and the
    /// first then loads from the wrong place.
    ///
    /// That was latent rather than theoretical -- this suite passed only
    /// through scheduling luck, and adding two more config-directory tests made
    /// it fail differently on every run.
    ///
    /// `serial_test` rather than a mutex local to this module, because
    /// `device.rs`, `paths.rs` and `auth.rs` redirect the same variable and are
    /// serialized the same way. A module-local lock coordinates none of those,
    /// so it would leave exactly the race it appears to fix.
    ///
    /// Use one spelling, not both: stacking the bare and qualified forms on the
    /// same test serializes it twice for no benefit and risks it waiting on a
    /// lock it already holds.
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
    #[serial_test::serial]
    fn launchd_spec_uses_program_arguments_without_shell() {
        let temp = TempDir::new().unwrap();
        let _guard = EnvVarGuard::set("TOKSCALE_CONFIG_DIR", temp.path());
        let settings = AutosubmitSettings {
            interval_minutes: 60,
            ..AutosubmitSettings::default()
        };
        let managed = temp.path().join("autosubmit").join(MANAGED_EXECUTABLE_NAME);
        let spec = render_launchd_spec_for_home(&managed, &settings, temp.path()).unwrap();
        assert_eq!(
            spec.files[0].0,
            temp.path()
                .join("Library")
                .join("LaunchAgents")
                .join("ai.tokscale.autosubmit.plist")
        );
        let content = &spec.files[0].1;
        assert!(content.contains(&format!("<string>{}</string>", managed.display())));
        assert!(content.contains("<string>autosubmit</string>"));
        assert!(content.contains("<string>run</string>"));
        assert!(content.contains("<key>RunAtLoad</key><true/>"));
        assert!(content.contains("<key>StartInterval</key><integer>3600</integer>"));
        assert!(!content.contains("/bin/sh"));
        assert!(!content.contains("launchctl load"));
        assert!(!content.contains("launchctl unload"));
    }

    #[test]
    fn launchd_commands_target_the_active_user_domain() {
        let service = launchd_service_for_uid(501);
        let plist = Path::new("/Users/alice/Library/LaunchAgents/ai.tokscale.autosubmit.plist");

        assert_eq!(service.domain, "gui/501");
        assert_eq!(service.target, "gui/501/ai.tokscale.autosubmit");
        assert_eq!(
            launchd_bootstrap_command(&service, plist),
            (
                "launchctl".to_string(),
                vec![
                    "bootstrap".to_string(),
                    "gui/501".to_string(),
                    plist.to_string_lossy().into_owned()
                ]
            )
        );
        assert_eq!(
            launchd_bootout_command(&service),
            (
                "launchctl".to_string(),
                vec![
                    "bootout".to_string(),
                    "--wait".to_string(),
                    "gui/501/ai.tokscale.autosubmit".to_string()
                ]
            )
        );
        assert_eq!(
            launchd_print_command(&service),
            (
                "launchctl".to_string(),
                vec![
                    "print".to_string(),
                    "gui/501/ai.tokscale.autosubmit".to_string()
                ]
            )
        );
    }

    #[test]
    fn launchd_verification_requires_the_exact_service_target() {
        let service = launchd_service_for_uid(501);
        let verified = CapturedCommand {
            command: "launchctl print gui/501/ai.tokscale.autosubmit".to_string(),
            status: "exit status: 0".to_string(),
            success: true,
            stdout: "gui/501/ai.tokscale.autosubmit = {\n}".to_string(),
            stderr: String::new(),
        };
        verify_launchd_service(&service, &verified).unwrap();

        let unverified = CapturedCommand {
            command: "launchctl print gui/501/ai.tokscale.autosubmit".to_string(),
            status: "exit status: 0".to_string(),
            success: true,
            stdout: "gui/501/another.service = {\n}".to_string(),
            stderr: "target was not found".to_string(),
        };
        let error = verify_launchd_service(&service, &unverified).unwrap_err();
        let rendered = error.to_string();
        assert!(rendered.contains("launchctl print gui/501/ai.tokscale.autosubmit"));
        assert!(rendered.contains("exit status: 0"));
        assert!(rendered.contains("gui/501/another.service"));
        assert!(rendered.contains("target was not found"));
    }

    #[test]
    fn launchd_absence_is_idempotent_only_for_the_service_target() {
        let service = launchd_service_for_uid(501);
        let absent = CapturedCommand {
            command: "launchctl print gui/501/ai.tokscale.autosubmit".to_string(),
            status: "exit status: 3".to_string(),
            success: false,
            stdout: String::new(),
            stderr: "Could not find service \"gui/501/ai.tokscale.autosubmit\"".to_string(),
        };
        assert!(launchd_service_is_absent(&service, &absent));

        let unrelated = CapturedCommand {
            stderr: "Could not find service \"gui/501/other.service\"".to_string(),
            ..absent
        };
        assert!(!launchd_service_is_absent(&service, &unrelated));
    }

    #[test]
    // `render_systemd_spec` resolves the unit directory from `XDG_CONFIG_HOME`
    // and the log path from the config dir (`TOKSCALE_CONFIG_DIR`), then
    // creates that directory. Both are process-global and every sibling test
    // that sets them is serial, so running this one in parallel with them
    // reads a half-torn override — the failure surfaces as an `unwrap` on the
    // `Err` from `create_dir_all`.
    #[serial_test::serial]
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
    #[serial_test::serial]
    fn scheduler_specs_use_the_managed_executable() {
        let temp = TempDir::new().unwrap();
        let _guard = EnvVarGuard::set("TOKSCALE_CONFIG_DIR", temp.path());
        let managed = managed_scheduler_executable_path().unwrap();
        let process_executable = Path::new("/workspace/node_modules/.bin/tokscale");
        let settings = AutosubmitSettings {
            interval_minutes: 60,
            ..AutosubmitSettings::default()
        };

        for scheduler in [
            SchedulerKind::Launchd,
            SchedulerKind::Systemd,
            SchedulerKind::Cron,
            SchedulerKind::WindowsTaskScheduler,
        ] {
            let spec = render_scheduler_spec(scheduler, &managed, &settings).unwrap();
            // Two escaping layers stack here, and #1007 only accounted for
            // one, so this still failed on Windows.
            //
            // Layer 1 is the renderer: `systemd_escape_path` doubles every
            // `\` (and turns spaces into `\x20`) *before* the value reaches
            // the unit file, while every other renderer stores the path
            // verbatim. Layer 2 is `Debug`, which doubles whatever it finds.
            // A Windows path therefore appears as `\\\\` in the systemd spec
            // and `\\` in the other three; a needle escaped once matches
            // three of the four and silently fails the fourth.
            //
            // Deriving the needle through the renderer's own escaper keeps
            // the two in step rather than restating its rules here.
            let stored = match scheduler {
                SchedulerKind::Systemd => systemd_escape_path(&managed),
                _ => managed.to_string_lossy().into_owned(),
            };
            let rendered = format!("{spec:?}");
            let needle = stored.replace('\\', "\\\\");
            assert!(
                rendered.contains(&needle),
                "{scheduler:?} spec does not reference the managed executable\n\
                 needle: {needle}\nrendered: {rendered}"
            );
            let absent = process_executable.to_string_lossy().replace('\\', "\\\\");
            assert!(!rendered.contains(&absent));
        }
    }

    #[cfg(unix)]
    #[test]
    #[serial_test::serial]
    fn managed_executable_refreshes_atomically_with_source_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let temp = TempDir::new().unwrap();
        let _guard = EnvVarGuard::set("TOKSCALE_CONFIG_DIR", temp.path());
        let source = temp.path().join("tokscale-source");
        fs::write(&source, "first binary").unwrap();
        fs::set_permissions(&source, fs::Permissions::from_mode(0o755)).unwrap();

        let managed_destination = next_managed_scheduler_executable_path().unwrap();
        let managed = prepare_managed_scheduler_executable(&source, managed_destination).unwrap();
        assert_eq!(
            managed,
            temp.path().join("autosubmit").join(MANAGED_EXECUTABLE_NAME)
        );
        assert_eq!(fs::read_to_string(&managed).unwrap(), "first binary");
        assert_eq!(
            fs::metadata(&managed).unwrap().permissions().mode() & 0o777,
            0o755
        );

        fs::write(&source, "second binary").unwrap();
        fs::set_permissions(&source, fs::Permissions::from_mode(0o711)).unwrap();
        prepare_managed_scheduler_executable(&source, managed.clone()).unwrap();

        assert_eq!(fs::read_to_string(&managed).unwrap(), "second binary");
        assert_eq!(
            fs::metadata(&managed).unwrap().permissions().mode() & 0o777,
            0o711
        );
        assert!(fs::read_dir(managed.parent().unwrap())
            .unwrap()
            .all(|entry| !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".tmp")));
    }

    #[cfg(unix)]
    #[test]
    #[serial_test::serial]
    fn managed_executable_rejects_nonexecutable_source() {
        use std::os::unix::fs::PermissionsExt;

        let temp = TempDir::new().unwrap();
        let _guard = EnvVarGuard::set("TOKSCALE_CONFIG_DIR", temp.path());
        let source = temp.path().join("tokscale-source");
        fs::write(&source, "not executable").unwrap();
        fs::set_permissions(&source, fs::Permissions::from_mode(0o644)).unwrap();

        let error = prepare_managed_scheduler_executable(
            &source,
            next_managed_scheduler_executable_path().unwrap(),
        )
        .unwrap_err();
        assert!(error.to_string().contains("not executable"));
    }

    #[cfg(unix)]
    #[test]
    #[serial_test::serial]
    fn windows_reenable_renders_a_new_versioned_managed_executable_path() {
        use std::os::unix::fs::PermissionsExt;

        let temp = TempDir::new().unwrap();
        let _guard = EnvVarGuard::set("TOKSCALE_CONFIG_DIR", temp.path());
        let managed_dir = temp.path().join("autosubmit");
        let existing = versioned_windows_managed_executable_path(&managed_dir, 1, 1);
        let replacement = versioned_windows_managed_executable_path(&managed_dir, 2, 2);
        fs::create_dir_all(&managed_dir).unwrap();
        fs::write(&existing, "old executable").unwrap();
        let source = temp.path().join("tokscale-source");
        fs::write(&source, "new executable").unwrap();
        fs::set_permissions(&source, fs::Permissions::from_mode(0o755)).unwrap();

        let prepared = prepare_managed_scheduler_executable(&source, replacement.clone()).unwrap();
        let settings = AutosubmitSettings {
            interval_minutes: 60,
            ..AutosubmitSettings::default()
        };
        let existing_task = render_windows_task_spec(&existing, &settings).unwrap();
        let replacement_task = render_windows_task_spec(&prepared, &settings).unwrap();

        assert_eq!(fs::read_to_string(&existing).unwrap(), "old executable");
        assert_eq!(fs::read_to_string(&prepared).unwrap(), "new executable");
        assert_ne!(existing, prepared);
        assert!(existing.starts_with(&managed_dir));
        assert!(prepared.starts_with(&managed_dir));
        assert!(existing_task.install_commands[0]
            .1
            .iter()
            .any(|arg| arg.contains(&existing.to_string_lossy().into_owned())));
        assert!(replacement_task.install_commands[0]
            .1
            .iter()
            .any(|arg| arg.contains(&prepared.to_string_lossy().into_owned())));
        assert!(!replacement_task.install_commands[0]
            .1
            .iter()
            .any(|arg| arg.contains(&existing.to_string_lossy().into_owned())));
    }

    #[test]
    fn scheduler_cleanup_tolerates_only_known_absent_entries() {
        let absent_windows_task = CapturedCommand {
            command: format!("schtasks /Delete /F /TN {JOB_ID}"),
            status: "exit status: 1".to_string(),
            success: false,
            stdout: String::new(),
            stderr: "ERROR: The system cannot find the file specified.".to_string(),
        };
        assert!(scheduler_entry_is_absent(
            SchedulerKind::WindowsTaskScheduler,
            &absent_windows_task
        ));
        cleanup_scheduler_command_result(
            SchedulerKind::WindowsTaskScheduler,
            "schtasks delete",
            &absent_windows_task,
        )
        .unwrap();

        let absent_windows_task_by_name = CapturedCommand {
            stderr: format!(
                "ERROR: The specified task name \"{JOB_ID}\" does not exist in the system."
            ),
            ..absent_windows_task.clone()
        };
        assert!(scheduler_entry_is_absent(
            SchedulerKind::WindowsTaskScheduler,
            &absent_windows_task_by_name
        ));

        let absent_systemd_timer = CapturedCommand {
            command: format!("systemctl --user disable --now {SYSTEMD_TIMER_UNIT}"),
            status: "exit status: 1".to_string(),
            success: false,
            stdout: String::new(),
            stderr: format!(
                "Failed to disable unit: Unit file {SYSTEMD_TIMER_UNIT} does not exist."
            ),
        };
        assert!(scheduler_entry_is_absent(
            SchedulerKind::Systemd,
            &absent_systemd_timer
        ));

        let invalid_task_name = CapturedCommand {
            stderr: format!("ERROR: The specified task name \"{JOB_ID}\" is invalid."),
            ..absent_windows_task.clone()
        };
        assert!(!scheduler_entry_is_absent(
            SchedulerKind::WindowsTaskScheduler,
            &invalid_task_name
        ));

        let cleanup_failure = CapturedCommand {
            stderr: "Access is denied.".to_string(),
            ..absent_windows_task
        };
        assert!(!scheduler_entry_is_absent(
            SchedulerKind::WindowsTaskScheduler,
            &cleanup_failure
        ));
        assert!(cleanup_scheduler_command_result(
            SchedulerKind::WindowsTaskScheduler,
            "schtasks delete",
            &cleanup_failure,
        )
        .is_err());
    }

    #[test]
    fn windows_managed_executable_cleanup_recognizes_versioned_and_legacy_names() {
        assert!(is_windows_managed_executable_name("tokscale.exe"));
        assert!(is_windows_managed_executable_name("tokscale-123-456-7.exe"));
        assert!(!is_windows_managed_executable_name("tokscale.exe.tmp"));
        assert!(!is_windows_managed_executable_name("unrelated.exe"));
    }

    fn enable_args(scheduler: SchedulerKind) -> AutosubmitEnableArgs {
        AutosubmitEnableArgs {
            interval: "1h".to_string(),
            clients: ClientFlags::default(),
            date: DateRangeFlags::default(),
            scheduler: Some(scheduler),
        }
    }

    #[test]
    #[serial_test::serial]
    fn windows_reenable_failure_keeps_the_existing_task_intact() {
        use std::cell::Cell;

        let temp = TempDir::new().unwrap();
        let _guard = EnvVarGuard::set("TOKSCALE_CONFIG_DIR", temp.path());
        let mut settings = crate::tui::settings::Settings::load();
        settings.autosubmit = AutosubmitSettings {
            enabled: true,
            scheduler: Some(SchedulerKind::WindowsTaskScheduler.as_str().to_string()),
            ..AutosubmitSettings::default()
        };
        settings.save().unwrap();
        let uninstall_calls = Cell::new(0);

        let error = enable_with_scheduler_operations(
            enable_args(SchedulerKind::WindowsTaskScheduler),
            |_, _, _| Err(anyhow::anyhow!("schtasks replacement failed")),
            |_, _, _| {
                uninstall_calls.set(uninstall_calls.get() + 1);
                Ok(())
            },
        )
        .unwrap_err();

        assert!(error.to_string().contains("schtasks replacement failed"));
        assert_eq!(uninstall_calls.get(), 0);
        let restored = crate::tui::settings::Settings::load().autosubmit;
        assert!(restored.enabled);
        assert_eq!(
            restored.scheduler.as_deref(),
            Some(SchedulerKind::WindowsTaskScheduler.as_str())
        );
    }

    #[test]
    #[serial_test::serial]
    fn scheduler_snapshot_preserves_versioned_windows_executable_path() {
        let temp = TempDir::new().unwrap();
        let _guard = EnvVarGuard::set("TOKSCALE_CONFIG_DIR", temp.path());
        let executable = temp
            .path()
            .join("autosubmit")
            .join("tokscale-123-456-7.exe");
        let settings = AutosubmitSettings {
            scheduler: Some(SchedulerKind::WindowsTaskScheduler.as_str().to_string()),
            managed_executable: Some(executable.to_string_lossy().into_owned()),
            ..AutosubmitSettings::default()
        };

        let snapshot =
            snapshot_scheduler_artifacts(SchedulerKind::WindowsTaskScheduler, &settings).unwrap();

        assert_eq!(snapshot.executable, executable);
    }

    #[test]
    #[serial_test::serial]
    fn disable_persists_settings_after_known_absent_scheduler_cleanup() {
        use std::cell::Cell;

        let temp = TempDir::new().unwrap();
        let _guard = EnvVarGuard::set("TOKSCALE_CONFIG_DIR", temp.path());
        let mut settings = crate::tui::settings::Settings::load();
        settings.autosubmit = AutosubmitSettings {
            enabled: true,
            scheduler: Some(SchedulerKind::WindowsTaskScheduler.as_str().to_string()),
            ..AutosubmitSettings::default()
        };
        settings.save().unwrap();
        let executable_cleanup_calls = Cell::new(0);

        disable_with_scheduler_operations(
            |scheduler, _| {
                let absent = CapturedCommand {
                    command: format!("schtasks /Delete /F /TN {JOB_ID}"),
                    status: "exit status: 1".to_string(),
                    success: false,
                    stdout: String::new(),
                    stderr: "ERROR: The system cannot find the file specified.".to_string(),
                };
                cleanup_scheduler_command_result(scheduler, "schtasks delete", &absent)
            },
            || {
                executable_cleanup_calls.set(executable_cleanup_calls.get() + 1);
                Ok(())
            },
        )
        .unwrap();

        assert!(!crate::tui::settings::Settings::load().autosubmit.enabled);
        assert_eq!(executable_cleanup_calls.get(), 1);
    }

    #[test]
    #[serial_test::serial]
    fn disable_retries_executable_cleanup_after_a_previous_locked_run() {
        use std::cell::Cell;

        let temp = TempDir::new().unwrap();
        let _guard = EnvVarGuard::set("TOKSCALE_CONFIG_DIR", temp.path());
        let mut settings = crate::tui::settings::Settings::load();
        settings.autosubmit = AutosubmitSettings {
            enabled: false,
            managed_executable: Some(
                temp.path()
                    .join("autosubmit")
                    .join("tokscale-123-456-7.exe")
                    .to_string_lossy()
                    .into_owned(),
            ),
            ..AutosubmitSettings::default()
        };
        settings.save().unwrap();
        let executable_cleanup_calls = Cell::new(0);

        disable_with_scheduler_operations(
            |_, _| panic!("disabled autosubmit must not uninstall a scheduler"),
            || {
                executable_cleanup_calls.set(executable_cleanup_calls.get() + 1);
                Ok(())
            },
        )
        .unwrap();

        assert_eq!(executable_cleanup_calls.get(), 1);
        assert!(crate::tui::settings::Settings::load()
            .autosubmit
            .managed_executable
            .is_none());
    }

    #[test]
    #[serial_test::serial]
    fn disable_keeps_settings_enabled_after_scheduler_cleanup_failure() {
        let temp = TempDir::new().unwrap();
        let _guard = EnvVarGuard::set("TOKSCALE_CONFIG_DIR", temp.path());
        let mut settings = crate::tui::settings::Settings::load();
        settings.autosubmit = AutosubmitSettings {
            enabled: true,
            scheduler: Some(SchedulerKind::Systemd.as_str().to_string()),
            last_error: Some("previous error".to_string()),
            ..AutosubmitSettings::default()
        };
        settings.save().unwrap();

        let error = disable_with_scheduler_operations(
            |_, _| Err(anyhow::anyhow!("systemctl cleanup failed")),
            || panic!("managed executable cleanup must not run after scheduler cleanup failure"),
        )
        .unwrap_err();

        assert!(error.to_string().contains("systemctl cleanup failed"));
        let restored = crate::tui::settings::Settings::load().autosubmit;
        assert!(restored.enabled);
        assert_eq!(restored.last_error.as_deref(), Some("previous error"));
    }

    #[test]
    #[serial_test::serial]
    fn enable_persists_settings_before_scheduler_installation() {
        use std::cell::Cell;

        let temp = TempDir::new().unwrap();
        let _guard = EnvVarGuard::set("TOKSCALE_CONFIG_DIR", temp.path());
        let observed_enabled = Cell::new(false);

        enable_with_scheduler_operations(
            enable_args(SchedulerKind::Cron),
            |_, _, _| {
                observed_enabled.set(crate::tui::settings::Settings::load().autosubmit.enabled);
                Ok(())
            },
            |_, _, _| Ok(()),
        )
        .unwrap();

        assert!(observed_enabled.get());
        assert!(crate::tui::settings::Settings::load().autosubmit.enabled);
    }

    #[test]
    #[serial_test::serial]
    fn enable_rolls_back_settings_and_managed_executable_after_activation_failure() {
        use std::cell::Cell;

        let temp = TempDir::new().unwrap();
        let _guard = EnvVarGuard::set("TOKSCALE_CONFIG_DIR", temp.path());
        let cleanup_calls = Cell::new(0);

        let error = enable_with_scheduler_operations(
            enable_args(SchedulerKind::Cron),
            |_, _, _| Err(anyhow::anyhow!("scheduler activation failed")),
            |_, _, _| {
                cleanup_calls.set(cleanup_calls.get() + 1);
                Ok(())
            },
        )
        .unwrap_err();

        assert!(error.to_string().contains("scheduler activation failed"));
        assert_eq!(cleanup_calls.get(), 1);
        let restored = crate::tui::settings::Settings::load().autosubmit;
        assert!(!restored.enabled);
        assert!(restored.scheduler.is_none());
        assert!(!managed_scheduler_executable_path().unwrap().exists());
    }

    #[test]
    #[serial_test::serial]
    fn enable_rolls_back_after_post_bootstrap_verification_failure() {
        let temp = TempDir::new().unwrap();
        let _guard = EnvVarGuard::set("TOKSCALE_CONFIG_DIR", temp.path());

        let error = enable_with_scheduler_operations(
            enable_args(SchedulerKind::Launchd),
            |_, _, _| {
                Err(anyhow::anyhow!(
                    "launchd print verification failed: exact service target was absent"
                ))
            },
            |_, _, _| Ok(()),
        )
        .unwrap_err();

        assert!(error
            .to_string()
            .contains("launchd print verification failed"));
        assert!(!crate::tui::settings::Settings::load().autosubmit.enabled);
        assert!(!managed_scheduler_executable_path().unwrap().exists());
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

    /// Scheduled autosubmit is the one date-filtered path that *submits*, so a
    /// "today" resolved from the host instead of the pinned zone selects a
    /// partial day out of correctly-keyed buckets — and the server's monotonic
    /// per-day guard then freezes that low value forever.
    ///
    /// Pacific/Kiritimati (UTC+14) and Pacific/Niue (UTC-11) are 25 hours
    /// apart, so they are never on the same calendar date. Reading
    /// `chrono::Local` would hand back the same day for both pins; only reading
    /// the pin can tell them apart.
    #[test]
    #[serial_test::serial]
    fn submit_filters_resolve_today_in_the_pinned_bucket_timezone() {
        fn today_for_pinned_zone(zone: &str) -> Option<String> {
            let temp = TempDir::new().unwrap();
            std::fs::write(
                temp.path().join("settings.json"),
                format!(r#"{{"scanner":{{"bucketTimezone":"{zone}"}}}}"#),
            )
            .unwrap();
            let _guard = EnvVarGuard::set("TOKSCALE_CONFIG_DIR", temp.path());

            let settings = AutosubmitSettings {
                today: true,
                ..AutosubmitSettings::default()
            };
            let (_, since, until, _) = submit_filters(&settings);
            assert_eq!(since, until, "--today is a single-day range");
            since
        }

        let kiritimati_zone =
            tokscale_core::BucketTimezone::from_pinned_name(Some("Pacific/Kiritimati"));
        // Bracket the call: `submit_filters` reads the clock itself, so a date
        // sampled only afterwards would disagree with it across a rollover.
        let before = kiritimati_zone.today();
        let kiritimati = today_for_pinned_zone("Pacific/Kiritimati");
        let after = kiritimati_zone.today();

        let niue = today_for_pinned_zone("Pacific/Niue");

        let acceptable: Vec<String> = [before, after]
            .iter()
            .map(|date| date.format("%Y-%m-%d").to_string())
            .collect();
        assert!(
            kiritimati
                .as_deref()
                .is_some_and(|day| acceptable.iter().any(|expected| expected == day)),
            "autosubmit must resolve today in the pinned zone, not the host's — \
             got {kiritimati:?}, expected one of {acceptable:?}"
        );
        assert_ne!(
            kiritimati, niue,
            "two pins 25 hours apart can never share a calendar date — equal \
             values mean the host clock was read instead of the pin"
        );
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

    /// `run_lock_blocks_concurrent_holder` above only exercises whatever the
    /// host reports, so it cannot see the Windows classification. These pin the
    /// error values directly: on Windows a contended lock arrives as
    /// `ERROR_LOCK_VIOLATION`, which is not `WouldBlock`, and treating it as a
    /// hard error made a scheduled run fail instead of yielding to the run
    /// already in progress.
    #[test]
    fn lock_contention_covers_every_platform_error() {
        // Whatever this host reports for a contended lock must classify as
        // contention — that is the contract `try_acquire_run_lock` relies on.
        assert!(is_lock_contention(&fs2::lock_contended_error()));

        assert!(is_lock_contention(&std::io::Error::from_raw_os_error(
            WINDOWS_ERROR_LOCK_VIOLATION
        )));
        assert!(is_lock_contention(&std::io::Error::from(
            ErrorKind::WouldBlock
        )));

        // A genuine failure must still surface rather than be silently read as
        // "someone else is running".
        assert!(!is_lock_contention(&std::io::Error::from(
            ErrorKind::PermissionDenied
        )));
        assert!(!is_lock_contention(&std::io::Error::from(
            ErrorKind::NotFound
        )));
    }
    /// The scheduler runs its own copy, so an upgrade that replaces the
    /// installed binary leaves it behind. These pin the detection, because the
    /// drift itself is silent by construction.
    #[test]
    fn managed_executable_stale_reports_a_version_change() {
        let settings = AutosubmitSettings {
            enabled: true,
            managed_executable: Some("/nonexistent/managed/tokscale".to_string()),
            managed_executable_version: Some("0.0.1-old".to_string()),
            ..AutosubmitSettings::default()
        };
        assert!(managed_executable_is_stale(&settings));
    }

    #[test]
    fn managed_executable_stale_is_quiet_when_versions_match() {
        let settings = AutosubmitSettings {
            enabled: true,
            managed_executable: Some("/nonexistent/managed/tokscale".to_string()),
            managed_executable_version: Some(RUNNING_VERSION.to_string()),
            ..AutosubmitSettings::default()
        };
        assert!(!managed_executable_is_stale(&settings));
    }

    #[test]
    fn managed_executable_stale_is_quiet_when_autosubmit_is_disabled() {
        // Nothing is scheduled, so a stale copy submits nothing and warning
        // about it would be noise.
        let settings = AutosubmitSettings {
            enabled: false,
            managed_executable: Some("/nonexistent/managed/tokscale".to_string()),
            managed_executable_version: Some("0.0.1-old".to_string()),
            ..AutosubmitSettings::default()
        };
        assert!(!managed_executable_is_stale(&settings));
    }

    #[test]
    fn managed_executable_stale_treats_a_missing_version_as_drift() {
        // Configs written before the field existed. The recorded build is
        // genuinely unknown, so claiming it is current would be a guess.
        let settings = AutosubmitSettings {
            enabled: true,
            managed_executable: Some("/nonexistent/managed/tokscale".to_string()),
            managed_executable_version: None,
            ..AutosubmitSettings::default()
        };
        assert!(managed_executable_is_stale(&settings));
    }

    #[test]
    fn managed_executable_stale_ignores_self_invocation() {
        // Running the managed copy compares it against itself, which always
        // matches and proves nothing -- a binary cannot observe its own
        // staleness. Deliberately reports no drift rather than a false all-clear
        // derived from the version mismatch below.
        let running = std::env::current_exe().unwrap();
        let settings = AutosubmitSettings {
            enabled: true,
            managed_executable: Some(running.to_string_lossy().into_owned()),
            managed_executable_version: Some("0.0.1-old".to_string()),
            ..AutosubmitSettings::default()
        };
        assert!(!managed_executable_is_stale(&settings));
    }

    #[test]
    fn status_output_surfaces_the_scheduled_build() {
        let settings = AutosubmitSettings {
            enabled: true,
            managed_executable: Some("/nonexistent/managed/tokscale".to_string()),
            managed_executable_version: Some("0.0.1-old".to_string()),
            ..AutosubmitSettings::default()
        };
        let output = status_output(&settings);
        assert_eq!(
            output.managed_executable.as_deref(),
            Some("/nonexistent/managed/tokscale")
        );
        assert_eq!(
            output.managed_executable_version.as_deref(),
            Some("0.0.1-old")
        );
        assert!(
            output.managed_executable_stale,
            "scripted checks read the boolean rather than diffing versions themselves"
        );
    }

    #[test]
    fn refresh_command_preserves_a_non_default_configuration() {
        // `enable` rebuilds every field from its arguments and `--interval`
        // defaults to 24h, so advising a bare re-run would turn a 30m interval
        // into 24h and drop the filters. The printed command has to be safe to
        // paste verbatim.
        let settings = AutosubmitSettings {
            enabled: true,
            interval_minutes: 30,
            clients: vec!["codex".to_string(), "claude".to_string()],
            since: Some("2026-01-01".to_string()),
            week: true,
            scheduler: Some(SchedulerKind::Launchd.as_str().to_string()),
            ..AutosubmitSettings::default()
        };
        assert_eq!(
            enable_command_for(&settings),
            "tokscale autosubmit enable --interval 30m --client codex,claude \
             --since 2026-01-01 --week --scheduler launchd"
        );
    }

    #[test]
    fn refresh_command_omits_unset_options() {
        let settings = AutosubmitSettings {
            enabled: true,
            interval_minutes: 1440,
            ..AutosubmitSettings::default()
        };
        assert_eq!(
            enable_command_for(&settings),
            "tokscale autosubmit enable --interval 1440m"
        );
    }

    #[test]
    #[serial_test::serial]
    fn enable_withholds_the_version_until_the_scheduler_is_installed() {
        use std::cell::Cell;

        // On Windows a re-enable writes a freshly versioned copy while the task
        // still points at the previous one, so a process killed between the
        // first settings save and the scheduler install would leave settings
        // claiming a build the scheduler is not running -- and the stale check
        // would report clean at exactly the moment it is wrong. Recording the
        // version only after installation means an interrupted enable leaves
        // `None`, which reads as unknown and reports drift.
        let temp = TempDir::new().unwrap();
        let _guard = EnvVarGuard::set("TOKSCALE_CONFIG_DIR", temp.path());
        let observed = Cell::new(Some(String::new()));

        enable_with_scheduler_operations(
            enable_args(SchedulerKind::Cron),
            |_, _, _| {
                observed.set(
                    crate::tui::settings::Settings::load()
                        .autosubmit
                        .managed_executable_version,
                );
                Ok(())
            },
            |_, _, _| Ok(()),
        )
        .unwrap();

        assert_eq!(
            observed.take(),
            None,
            "the version must not be persisted before the scheduler points at the new copy"
        );
        assert_eq!(
            crate::tui::settings::Settings::load()
                .autosubmit
                .managed_executable_version
                .as_deref(),
            Some(RUNNING_VERSION),
            "and must be persisted once installation succeeds"
        );
    }

    #[test]
    #[serial_test::serial]
    fn enable_records_the_running_version_beside_the_managed_copy() {
        let temp = TempDir::new().unwrap();
        let _guard = EnvVarGuard::set("TOKSCALE_CONFIG_DIR", temp.path());

        enable_with_scheduler_operations(
            enable_args(SchedulerKind::Cron),
            |_, _, _| Ok(()),
            |_, _, _| Ok(()),
        )
        .unwrap();

        let saved = crate::tui::settings::Settings::load().autosubmit;
        assert_eq!(
            saved.managed_executable_version.as_deref(),
            Some(RUNNING_VERSION),
            "without this the next run cannot tell a stale scheduled job from a current one"
        );
        assert!(saved.managed_executable.is_some());
    }

    #[test]
    #[serial_test::serial]
    fn disable_clears_the_recorded_managed_version() {
        let temp = TempDir::new().unwrap();
        let _guard = EnvVarGuard::set("TOKSCALE_CONFIG_DIR", temp.path());
        let mut settings = crate::tui::settings::Settings::load();
        settings.autosubmit = AutosubmitSettings {
            enabled: true,
            scheduler: Some(SchedulerKind::Cron.as_str().to_string()),
            managed_executable: Some("/nonexistent/managed/tokscale".to_string()),
            managed_executable_version: Some("0.0.1-old".to_string()),
            ..AutosubmitSettings::default()
        };
        settings.save().unwrap();

        disable_with_scheduler_operations(|_, _| Ok(()), || Ok(())).unwrap();

        let saved = crate::tui::settings::Settings::load().autosubmit;
        assert_eq!(saved.managed_executable, None);
        assert_eq!(
            saved.managed_executable_version, None,
            "a stale version left behind would make the next enable look like drift"
        );
    }
}
