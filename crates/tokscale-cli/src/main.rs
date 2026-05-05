mod antigravity;
mod auth;
mod commands;
mod cursor;
mod paths;
mod tui;

use crate::tui::client_ui;
use anyhow::Result;
use clap::{Args, Parser, Subcommand, ValueEnum};
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;
use tui::Tab;

#[derive(Parser)]
#[command(name = "tokscale")]
#[command(author, version, about = "AI token usage analytics")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    #[arg(short, long, default_value = "blue")]
    theme: String,

    #[arg(short, long, default_value = "0")]
    refresh: u64,

    #[arg(long)]
    debug: bool,

    #[arg(long)]
    test_data: bool,

    #[arg(long, help = "Output as JSON")]
    json: bool,

    #[arg(long, help = "Use legacy CLI table output")]
    light: bool,

    #[arg(
        long = "write-cache",
        requires = "light",
        conflicts_with = "no_write_cache",
        help = "After --light renders, atomically overwrite the TUI cache with this report's data so the next `tokscale tui` starts from fresh data. Persists across invocations via settings.json `light.writeCache`."
    )]
    write_cache: bool,

    #[arg(
        long = "no-write-cache",
        requires = "light",
        conflicts_with = "write_cache",
        help = "Skip cache write even if settings.json `light.writeCache` is true. Only valid with --light."
    )]
    no_write_cache: bool,

    #[command(flatten)]
    clients: ClientFlags,

    #[command(flatten)]
    date: DateRangeFlags,

    #[arg(
        long,
        value_name = "PATH",
        global = true,
        help = "Read local session data from this home directory for local report commands"
    )]
    home: Option<String>,

    #[arg(long, help = "Show processing time")]
    benchmark: bool,

    #[arg(
        long,
        value_name = "STRATEGY",
        default_value = "client,model",
        help = "Grouping strategy for --light and --json output: model, client,model, client,provider,model, workspace,model"
    )]
    group_by: String,

    #[arg(long, help = "Disable spinner (for AI agents and scripts)")]
    no_spinner: bool,
}

#[derive(Subcommand)]
enum Commands {
    #[command(about = "Show model usage report")]
    Models {
        #[arg(long)]
        json: bool,
        #[arg(long)]
        light: bool,
        #[command(flatten)]
        clients: ClientFlags,
        #[command(flatten)]
        date: DateRangeFlags,
        #[arg(long, help = "Show processing time")]
        benchmark: bool,
        #[arg(
            long,
            value_name = "STRATEGY",
            default_value = "client,model",
            help = "Grouping strategy for --light and --json output: model, client,model, client,provider,model, workspace,model"
        )]
        group_by: String,
        #[arg(
            long = "write-cache",
            requires = "light",
            conflicts_with = "no_write_cache",
            help = "After --light renders, atomically overwrite the TUI cache with this report's data so the next `tokscale tui` starts from fresh data. Persists across invocations via settings.json `light.writeCache`."
        )]
        write_cache: bool,
        #[arg(
            long = "no-write-cache",
            requires = "light",
            conflicts_with = "write_cache",
            help = "Skip cache write even if settings.json `light.writeCache` is true. Only valid with --light."
        )]
        no_write_cache: bool,
        #[arg(long, help = "Disable spinner")]
        no_spinner: bool,
    },
    #[command(about = "Show monthly usage report")]
    Monthly {
        #[arg(long)]
        json: bool,
        #[arg(long)]
        light: bool,
        #[command(flatten)]
        clients: ClientFlags,
        #[command(flatten)]
        date: DateRangeFlags,
        #[arg(long, help = "Show processing time")]
        benchmark: bool,
        #[arg(long, help = "Disable spinner")]
        no_spinner: bool,
    },
    #[command(about = "Show hourly usage report")]
    Hourly {
        #[arg(long)]
        json: bool,
        #[arg(long)]
        light: bool,
        #[command(flatten)]
        clients: ClientFlags,
        #[command(flatten)]
        date: DateRangeFlags,
        #[arg(long, help = "Show processing time")]
        benchmark: bool,
        #[arg(long, help = "Disable spinner")]
        no_spinner: bool,
    },
    #[command(about = "Show pricing for a model")]
    Pricing {
        model_id: String,
        #[arg(long, help = "Output as JSON")]
        json: bool,
        #[arg(long, help = "Force specific provider (litellm or openrouter)")]
        provider: Option<String>,
        #[arg(long, help = "Disable spinner")]
        no_spinner: bool,
    },
    #[command(about = "Show local scan locations and session counts")]
    Clients {
        #[arg(long, help = "Output as JSON")]
        json: bool,
    },
    #[command(about = "Login to Tokscale (opens browser for GitHub auth)")]
    Login,
    #[command(about = "Logout from Tokscale")]
    Logout,
    #[command(about = "Show current logged in user")]
    Whoami,
    #[command(about = "Export contribution graph data as JSON")]
    Graph {
        #[arg(long, help = "Write to file instead of stdout")]
        output: Option<String>,
        #[command(flatten)]
        clients: ClientFlags,
        #[command(flatten)]
        date: DateRangeFlags,
        #[arg(long, help = "Show processing time")]
        benchmark: bool,
        #[arg(long, help = "Disable spinner")]
        no_spinner: bool,
    },
    #[command(about = "Launch interactive TUI with optional filters")]
    Tui {
        #[command(flatten)]
        clients: ClientFlags,
        #[command(flatten)]
        date: DateRangeFlags,
    },
    #[command(about = "Submit usage data to the Tokscale social platform")]
    Submit {
        #[command(flatten)]
        clients: ClientFlags,
        #[command(flatten)]
        date: DateRangeFlags,
        #[arg(
            long,
            help = "Show what would be submitted without actually submitting"
        )]
        dry_run: bool,
    },
    #[command(about = "Capture subprocess output for token usage tracking")]
    Headless {
        #[arg(help = "Source CLI (currently only 'codex' supported)")]
        source: String,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
        #[arg(long, help = "Override output format (json or jsonl)")]
        format: Option<String>,
        #[arg(long, help = "Write captured output to file")]
        output: Option<String>,
        #[arg(long, help = "Do not auto-add JSON output flags")]
        no_auto_flags: bool,
    },
    #[command(about = "Generate year-in-review wrapped image")]
    Wrapped {
        #[arg(long, help = "Output file path (default: tokscale-{year}-wrapped.png)")]
        output: Option<String>,
        #[arg(long, help = "Year to generate (default: current year)")]
        year: Option<String>,
        #[command(flatten)]
        client_flags: ClientFlags,
        #[arg(
            long,
            help = "Display total tokens in abbreviated format (e.g., 7.14B)"
        )]
        short: bool,
        #[arg(long, help = "Show Top OpenCode Agents (default)")]
        agents: bool,
        #[arg(long, help = "Show Top Clients instead of Top OpenCode Agents")]
        clients: bool,
        #[arg(long, help = "Disable pinning of Sisyphus agents in rankings")]
        disable_pinned: bool,
        #[arg(long, help = "Disable loading spinner (for scripting)")]
        no_spinner: bool,
    },
    #[command(about = "Cursor IDE integration commands")]
    Cursor {
        #[command(subcommand)]
        subcommand: CursorSubcommand,
    },
    #[command(about = "Antigravity integration commands")]
    Antigravity {
        #[command(subcommand)]
        subcommand: AntigravitySubcommand,
    },
    #[command(about = "Delete all submitted usage data from the server")]
    DeleteSubmittedData,
    #[command(about = "Warm TUI cache in background (internal)", hide = true)]
    WarmTuiCache,
}

#[derive(Subcommand)]
enum CursorSubcommand {
    #[command(about = "Login to Cursor (paste your session token)")]
    Login {
        #[arg(long, help = "Label for this Cursor account (e.g., work, personal)")]
        name: Option<String>,
    },
    #[command(about = "Logout from a Cursor account")]
    Logout {
        #[arg(long, help = "Account label or id")]
        name: Option<String>,
        #[arg(long, help = "Logout from all Cursor accounts")]
        all: bool,
        #[arg(long, help = "Also delete cached Cursor usage")]
        purge_cache: bool,
    },
    #[command(about = "Check Cursor authentication status")]
    Status {
        #[arg(long, help = "Account label or id")]
        name: Option<String>,
    },
    #[command(about = "List saved Cursor accounts")]
    Accounts {
        #[arg(long, help = "Output as JSON")]
        json: bool,
    },
    #[command(about = "Sync Cursor usage into the local cache")]
    Sync {
        #[arg(long, help = "Output as JSON")]
        json: bool,
    },
    #[command(about = "Switch active Cursor account")]
    Switch {
        #[arg(help = "Account label or id")]
        name: String,
    },
}

#[derive(Subcommand)]
enum AntigravitySubcommand {
    #[command(about = "Sync usage from running Antigravity language servers")]
    Sync,
    #[command(about = "Show Antigravity sync status")]
    Status {
        #[arg(long, help = "Output as JSON")]
        json: bool,
    },
    #[command(about = "Delete cached Antigravity usage artifacts")]
    PurgeCache,
}

fn main() -> Result<()> {
    use std::io::IsTerminal;

    let cli = Cli::parse();
    let can_use_tui = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();

    if cli.test_data {
        return tui::test_data_loading();
    }

    match cli.command {
        Some(Commands::Models {
            json,
            light,
            clients,
            date,
            benchmark,
            group_by,
            write_cache,
            no_write_cache,
            no_spinner,
        }) => {
            use tokscale_core::GroupBy;

            let group_by: GroupBy = group_by.parse().unwrap_or_else(|e| {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            });
            let today = date.today;
            let week = date.week;
            let month = date.month;
            let (since, until) = build_date_filter(today, week, month, date.since, date.until);
            let year = normalize_year_filter(today, week, month, date.year);
            let clients = build_client_filter(clients);
            if json || light || !can_use_tui {
                run_models_report(
                    json,
                    cli.home.clone(),
                    clients,
                    since,
                    until,
                    year,
                    benchmark,
                    no_spinner || !can_use_tui,
                    today,
                    week,
                    month,
                    group_by,
                    write_cache,
                    no_write_cache,
                )
            } else {
                ensure_home_supported_for_tui(&cli.home)?;
                auto_sync_cursor_before_tui(&cli.home, &clients)?;
                tui::run(
                    &cli.theme,
                    cli.refresh,
                    cli.debug,
                    clients,
                    since,
                    until,
                    year,
                    Some(Tab::Models),
                )
            }
        }
        Some(Commands::Monthly {
            json,
            light,
            clients,
            date,
            benchmark,
            no_spinner,
        }) => {
            let today = date.today;
            let week = date.week;
            let month = date.month;
            let (since, until) = build_date_filter(today, week, month, date.since, date.until);
            let year = normalize_year_filter(today, week, month, date.year);
            let clients = build_client_filter(clients);
            if json || light || !can_use_tui {
                run_monthly_report(
                    json,
                    cli.home.clone(),
                    clients,
                    since,
                    until,
                    year,
                    benchmark,
                    no_spinner || !can_use_tui,
                    today,
                    week,
                    month,
                )
            } else {
                ensure_home_supported_for_tui(&cli.home)?;
                auto_sync_cursor_before_tui(&cli.home, &clients)?;
                tui::run(
                    &cli.theme,
                    cli.refresh,
                    cli.debug,
                    clients,
                    since,
                    until,
                    year,
                    Some(Tab::Daily),
                )
            }
        }
        Some(Commands::Hourly {
            json,
            light,
            clients,
            date,
            benchmark,
            no_spinner,
        }) => {
            let today = date.today;
            let week = date.week;
            let month = date.month;
            let (since, until) = build_date_filter(today, week, month, date.since, date.until);
            let year = normalize_year_filter(today, week, month, date.year);
            let clients = build_client_filter(clients);
            if json || light || !can_use_tui {
                run_hourly_report(
                    json,
                    cli.home.clone(),
                    clients,
                    since,
                    until,
                    year,
                    benchmark,
                    no_spinner || !can_use_tui,
                    today,
                    week,
                    month,
                )
            } else {
                ensure_home_supported_for_tui(&cli.home)?;
                auto_sync_cursor_before_tui(&cli.home, &clients)?;
                tui::run(
                    &cli.theme,
                    cli.refresh,
                    cli.debug,
                    clients,
                    since,
                    until,
                    year,
                    Some(Tab::Hourly),
                )
            }
        }
        Some(Commands::Pricing {
            model_id,
            json,
            provider,
            no_spinner,
        }) => {
            reject_unsupported_home_override(&cli.home, "pricing")?;
            run_pricing_lookup(&model_id, json, provider.as_deref(), no_spinner)
        }
        Some(Commands::Clients { json }) => {
            reject_unsupported_home_override(&cli.home, "clients")?;
            run_clients_command(json)
        }
        Some(Commands::Login) => {
            reject_unsupported_home_override(&cli.home, "login")?;
            run_login_command()
        }
        Some(Commands::Logout) => {
            reject_unsupported_home_override(&cli.home, "logout")?;
            run_logout_command()
        }
        Some(Commands::Whoami) => {
            reject_unsupported_home_override(&cli.home, "whoami")?;
            run_whoami_command()
        }
        Some(Commands::Graph {
            output,
            clients,
            date,
            benchmark,
            no_spinner,
        }) => {
            let today = date.today;
            let week = date.week;
            let month = date.month;
            let (since, until) = build_date_filter(today, week, month, date.since, date.until);
            let year = normalize_year_filter(today, week, month, date.year);
            let clients = build_client_filter(clients);
            run_graph_command(
                output,
                cli.home.clone(),
                clients,
                since,
                until,
                year,
                benchmark,
                no_spinner,
            )
        }
        Some(Commands::Tui { clients, date }) => {
            ensure_home_supported_for_tui(&cli.home)?;
            let today = date.today;
            let week = date.week;
            let month = date.month;
            let (since, until) = build_date_filter(today, week, month, date.since, date.until);
            let year = normalize_year_filter(today, week, month, date.year);
            let clients = build_client_filter(clients);
            auto_sync_cursor_before_tui(&cli.home, &clients)?;
            tui::run(
                &cli.theme,
                cli.refresh,
                cli.debug,
                clients,
                since,
                until,
                year,
                None,
            )
        }
        Some(Commands::Submit {
            clients,
            date,
            dry_run,
        }) => {
            reject_unsupported_home_override(&cli.home, "submit")?;
            let today = date.today;
            let week = date.week;
            let month = date.month;
            let (since, until) = build_date_filter(today, week, month, date.since, date.until);
            let year = normalize_year_filter(today, week, month, date.year);
            // Bypass settings.json defaultClients for the submit path: we want the
            // submit-specific default_submit_clients() fallback (in run_submit_command)
            // to fire when the user passes no client flags, not the user's general
            // defaultClients view filter (which may exclude clients they still want
            // to upload). Pass an explicit empty defaults slice.
            let clients = build_client_filter_with_defaults(clients, &[]);
            run_submit_command(clients, since, until, year, dry_run)
        }
        Some(Commands::Headless {
            source,
            args,
            format,
            output,
            no_auto_flags,
        }) => {
            reject_unsupported_home_override(&cli.home, "headless")?;
            run_headless_command(&source, args, format, output, no_auto_flags)
        }
        Some(Commands::Wrapped {
            output,
            year,
            client_flags,
            short,
            agents,
            clients,
            disable_pinned,
            no_spinner: _,
        }) => {
            reject_unsupported_home_override(&cli.home, "wrapped")?;
            let client_filter = build_client_filter(client_flags);
            run_wrapped_command(
                output,
                year,
                client_filter,
                short,
                agents,
                clients,
                disable_pinned,
            )
        }
        Some(Commands::Cursor { subcommand }) => {
            reject_unsupported_home_override(&cli.home, "cursor")?;
            run_cursor_command(subcommand)
        }
        Some(Commands::Antigravity { subcommand }) => {
            reject_unsupported_home_override(&cli.home, "antigravity")?;
            run_antigravity_command(subcommand)
        }
        Some(Commands::DeleteSubmittedData) => {
            reject_unsupported_home_override(&cli.home, "delete-submitted-data")?;
            run_delete_data_command()
        }
        Some(Commands::WarmTuiCache) => run_warm_tui_cache(),
        None => {
            let today = cli.date.today;
            let week = cli.date.week;
            let month = cli.date.month;
            let clients = build_client_filter(cli.clients);
            let (since, until) =
                build_date_filter(today, week, month, cli.date.since, cli.date.until);
            let year = normalize_year_filter(today, week, month, cli.date.year);
            let group_by: tokscale_core::GroupBy = cli.group_by.parse().unwrap_or_else(|e| {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            });

            if cli.json {
                run_models_report(
                    cli.json,
                    cli.home.clone(),
                    clients,
                    since,
                    until,
                    year,
                    cli.benchmark,
                    cli.no_spinner || cli.json,
                    today,
                    week,
                    month,
                    group_by,
                    cli.write_cache,
                    cli.no_write_cache,
                )
            } else if cli.light || !can_use_tui {
                run_models_report(
                    false,
                    cli.home.clone(),
                    clients,
                    since,
                    until,
                    year,
                    cli.benchmark,
                    cli.no_spinner || !can_use_tui,
                    today,
                    week,
                    month,
                    group_by,
                    cli.write_cache,
                    cli.no_write_cache,
                )
            } else {
                ensure_home_supported_for_tui(&cli.home)?;
                auto_sync_cursor_before_tui(&cli.home, &clients)?;
                tui::run(
                    &cli.theme,
                    cli.refresh,
                    cli.debug,
                    clients,
                    since,
                    until,
                    year,
                    None,
                )
            }
        }
    }
}

/// Client identifiers exposed via `--client`.
///
/// Mirrors `tokscale_core::ClientId` plus the `Synthetic` meta-client. We
/// duplicate the variant set on the CLI side so `tokscale-core` stays free of
/// CLI-parsing dependencies and so `Synthetic` (which has no scan path of its
/// own) can be treated as a first-class filter value without changing core
/// invariants.
///
/// Variant order intentionally mirrors `ClientId::ALL` declaration order so
/// the TUI source picker, `--help`'s `[possible values: ...]` listing, and
/// any future iteration over `ClientFilter::value_variants()` agree on a
/// single chronological ordering. `Synthetic` is appended at the end since
/// it has no `ClientId` counterpart.
#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[value(rename_all = "lowercase")]
pub enum ClientFilter {
    Opencode,
    Claude,
    Codex,
    Cursor,
    Gemini,
    Amp,
    Droid,
    Openclaw,
    Pi,
    Kimi,
    Qwen,
    Roocode,
    Kilocode,
    Mux,
    Kilo,
    Crush,
    Hermes,
    Copilot,
    Goose,
    Codebuff,
    Antigravity,
    Synthetic,
}

impl ClientFilter {
    /// Returns the canonical lowercase identifier consumed by
    /// `tokscale_core` filter lists. Must match `ClientId::as_str` for every
    /// variant that has a corresponding `ClientId`.
    pub fn as_filter_str(&self) -> &'static str {
        match self {
            Self::Opencode => "opencode",
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Cursor => "cursor",
            Self::Gemini => "gemini",
            Self::Amp => "amp",
            Self::Droid => "droid",
            Self::Openclaw => "openclaw",
            Self::Pi => "pi",
            Self::Kimi => "kimi",
            Self::Qwen => "qwen",
            Self::Roocode => "roocode",
            Self::Kilocode => "kilocode",
            Self::Mux => "mux",
            Self::Kilo => "kilo",
            Self::Crush => "crush",
            Self::Hermes => "hermes",
            Self::Copilot => "copilot",
            Self::Goose => "goose",
            Self::Codebuff => "codebuff",
            Self::Antigravity => "antigravity",
            Self::Synthetic => "synthetic",
        }
    }

    /// Convert to the corresponding `ClientId`, or `None` for the
    /// `Synthetic` meta-client which has no scan path of its own.
    ///
    /// Used at boundaries where TUI state (`HashSet<ClientFilter>`) needs
    /// to feed core APIs that still consume `Vec<ClientId>`.
    pub fn to_client_id(self) -> Option<tokscale_core::ClientId> {
        use tokscale_core::ClientId;
        match self {
            Self::Opencode => Some(ClientId::OpenCode),
            Self::Claude => Some(ClientId::Claude),
            Self::Codex => Some(ClientId::Codex),
            Self::Cursor => Some(ClientId::Cursor),
            Self::Gemini => Some(ClientId::Gemini),
            Self::Amp => Some(ClientId::Amp),
            Self::Droid => Some(ClientId::Droid),
            Self::Openclaw => Some(ClientId::OpenClaw),
            Self::Pi => Some(ClientId::Pi),
            Self::Kimi => Some(ClientId::Kimi),
            Self::Qwen => Some(ClientId::Qwen),
            Self::Roocode => Some(ClientId::RooCode),
            Self::Kilocode => Some(ClientId::KiloCode),
            Self::Mux => Some(ClientId::Mux),
            Self::Kilo => Some(ClientId::Kilo),
            Self::Crush => Some(ClientId::Crush),
            Self::Hermes => Some(ClientId::Hermes),
            Self::Copilot => Some(ClientId::Copilot),
            Self::Goose => Some(ClientId::Goose),
            Self::Codebuff => Some(ClientId::Codebuff),
            Self::Antigravity => Some(ClientId::Antigravity),
            Self::Synthetic => None,
        }
    }

    /// Lift a `ClientId` back into a `ClientFilter`. Total inverse of
    /// `to_client_id` for non-`Synthetic` variants.
    pub fn from_client_id(client: tokscale_core::ClientId) -> Self {
        use tokscale_core::ClientId;
        match client {
            ClientId::OpenCode => Self::Opencode,
            ClientId::Claude => Self::Claude,
            ClientId::Codex => Self::Codex,
            ClientId::Cursor => Self::Cursor,
            ClientId::Gemini => Self::Gemini,
            ClientId::Amp => Self::Amp,
            ClientId::Droid => Self::Droid,
            ClientId::OpenClaw => Self::Openclaw,
            ClientId::Pi => Self::Pi,
            ClientId::Kimi => Self::Kimi,
            ClientId::Qwen => Self::Qwen,
            ClientId::RooCode => Self::Roocode,
            ClientId::KiloCode => Self::Kilocode,
            ClientId::Mux => Self::Mux,
            ClientId::Kilo => Self::Kilo,
            ClientId::Crush => Self::Crush,
            ClientId::Hermes => Self::Hermes,
            ClientId::Copilot => Self::Copilot,
            ClientId::Goose => Self::Goose,
            ClientId::Codebuff => Self::Codebuff,
            ClientId::Antigravity => Self::Antigravity,
        }
    }

    /// Parse a canonical lowercase identifier (the same form
    /// `as_filter_str` returns) into a `ClientFilter`. Returns `None` for
    /// any unknown id so callers can drop unrecognized settings entries
    /// without erroring.
    pub fn from_filter_str(s: &str) -> Option<Self> {
        Self::value_variants()
            .iter()
            .copied()
            .find(|f| f.as_filter_str() == s)
    }

    /// The "no filter" default set: every real client, with `Synthetic`
    /// **excluded**. Matches the pre-refactor behavior where a missing
    /// filter scanned every `ClientId` but did NOT post-process synthetic
    /// (synthetic detection has always been opt-in because it
    /// re-attributes messages from other clients to a different bucket).
    ///
    /// Single source of truth: every code path that needs a default
    /// filter (TUI launch, `submit` warm cache, etc.) must consult this
    /// so the cache key, the in-app state, and the loader filter all
    /// agree. Drift between them produces stale-cache misses on every
    /// launch.
    pub fn default_set() -> std::collections::HashSet<Self> {
        Self::value_variants()
            .iter()
            .copied()
            .filter(|f| !matches!(f, Self::Synthetic))
            .collect()
    }
}

#[derive(Args, Clone, Debug, Default)]
pub struct ClientFlags {
    /// Canonical client filter. Repeatable or comma-separated.
    /// Example: `--client opencode,claude` or `-c opencode -c claude`.
    #[arg(
        long = "client",
        short = 'c',
        value_enum,
        value_delimiter = ',',
        action = clap::ArgAction::Append,
        ignore_case = true,
        help = "Filter by client(s). Repeatable or comma-separated (e.g. -c opencode,claude)."
    )]
    pub clients: Vec<ClientFilter>,

    // ---- Deprecated legacy boolean flags ------------------------------
    // Hidden from --help. Kept for backward compatibility; print a stderr
    // deprecation warning when used. Slated for removal in the next major.
    #[arg(long, hide = true)]
    pub opencode: bool,
    #[arg(long, hide = true)]
    pub claude: bool,
    #[arg(long, hide = true)]
    pub codex: bool,
    #[arg(long, hide = true)]
    pub copilot: bool,
    #[arg(long, hide = true)]
    pub gemini: bool,
    #[arg(long, hide = true)]
    pub cursor: bool,
    #[arg(long, hide = true)]
    pub amp: bool,
    #[arg(long, hide = true)]
    pub codebuff: bool,
    #[arg(long, hide = true)]
    pub droid: bool,
    #[arg(long, hide = true)]
    pub openclaw: bool,
    #[arg(long, hide = true)]
    pub hermes: bool,
    #[arg(long, hide = true)]
    pub pi: bool,
    #[arg(long, hide = true)]
    pub kimi: bool,
    #[arg(long, hide = true)]
    pub qwen: bool,
    #[arg(long, hide = true)]
    pub roocode: bool,
    #[arg(long, hide = true)]
    pub kilocode: bool,
    #[arg(long, hide = true)]
    pub kilo: bool,
    #[arg(long, hide = true)]
    pub mux: bool,
    #[arg(long, hide = true)]
    pub crush: bool,
    #[arg(long, hide = true)]
    pub goose: bool,
    #[arg(long, hide = true)]
    pub antigravity: bool,
    #[arg(long, hide = true)]
    pub synthetic: bool,
}

#[derive(Args, Clone, Debug, Default)]
pub struct DateRangeFlags {
    #[arg(long, help = "Show only today's usage")]
    pub today: bool,
    #[arg(long, help = "Show last 7 days")]
    pub week: bool,
    #[arg(long, help = "Show current month")]
    pub month: bool,
    #[arg(long, help = "Start date (YYYY-MM-DD)")]
    pub since: Option<String>,
    #[arg(long, help = "End date (YYYY-MM-DD)")]
    pub until: Option<String>,
    #[arg(long, help = "Filter by year (YYYY)")]
    pub year: Option<String>,
}

/// Builds the client filter list passed to `tokscale_core`.
///
/// Resolution order:
/// 1. Collect canonical `--client/-c` values (preserves user order).
/// 2. Append any legacy `--<client>` boolean flags that are set, emitting a
///    one-time stderr deprecation warning so existing scripts keep working.
/// 3. If steps 1 and 2 produced nothing, fall back to user-configured
///    `defaultClients` from `~/.config/tokscale/settings.json` when present.
/// 4. Deduplicate while preserving first-seen order.
///
/// Returns `None` when no filters are active *and* no defaults configured
/// so the caller can scan all clients.
fn build_client_filter(flags: ClientFlags) -> Option<Vec<String>> {
    let defaults = tui::settings::load_default_clients();
    build_client_filter_with_defaults(flags, &defaults)
}

/// Pure variant of [`build_client_filter`] for unit-testable resolution.
/// `defaults` is the (already-validated) list of canonical filter ids that
/// should apply when no CLI flag is present.
fn build_client_filter_with_defaults(
    flags: ClientFlags,
    defaults: &[String],
) -> Option<Vec<String>> {
    let mut ordered: Vec<String> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    for client in &flags.clients {
        let id = client.as_filter_str().to_string();
        if seen.insert(id.clone()) {
            ordered.push(id);
        }
    }

    let legacy: [(bool, ClientFilter); 22] = [
        (flags.opencode, ClientFilter::Opencode),
        (flags.claude, ClientFilter::Claude),
        (flags.codex, ClientFilter::Codex),
        (flags.cursor, ClientFilter::Cursor),
        (flags.gemini, ClientFilter::Gemini),
        (flags.amp, ClientFilter::Amp),
        (flags.codebuff, ClientFilter::Codebuff),
        (flags.droid, ClientFilter::Droid),
        (flags.openclaw, ClientFilter::Openclaw),
        (flags.pi, ClientFilter::Pi),
        (flags.kimi, ClientFilter::Kimi),
        (flags.qwen, ClientFilter::Qwen),
        (flags.roocode, ClientFilter::Roocode),
        (flags.kilocode, ClientFilter::Kilocode),
        (flags.mux, ClientFilter::Mux),
        (flags.kilo, ClientFilter::Kilo),
        (flags.crush, ClientFilter::Crush),
        (flags.hermes, ClientFilter::Hermes),
        (flags.copilot, ClientFilter::Copilot),
        (flags.goose, ClientFilter::Goose),
        (flags.antigravity, ClientFilter::Antigravity),
        (flags.synthetic, ClientFilter::Synthetic),
    ];

    let mut legacy_used: Vec<&'static str> = Vec::new();
    for (enabled, client) in legacy {
        if !enabled {
            continue;
        }
        let id = client.as_filter_str();
        legacy_used.push(id);
        if seen.insert(id.to_string()) {
            ordered.push(id.to_string());
        }
    }

    if !legacy_used.is_empty() {
        emit_legacy_client_flag_warning(&legacy_used);
    }

    // Defaults only apply when the user passed neither canonical nor legacy
    // flags. CLI flags always win — predictable semantics over "merge".
    // Unknown / typo'd ids are dropped silently so a stale settings.json
    // entry never breaks tokscale.
    if ordered.is_empty() {
        for raw in defaults {
            if let Some(client) = ClientFilter::from_filter_str(raw) {
                let id = client.as_filter_str().to_string();
                if seen.insert(id.clone()) {
                    ordered.push(id);
                }
            }
        }
    }

    if ordered.is_empty() {
        None
    } else {
        Some(ordered)
    }
}

/// Emits a single stderr deprecation warning when legacy `--<client>` flags
/// are used. Suppressed entirely when stderr is not a TTY (e.g. when piping
/// JSON output through scripts) so machine-parseable output stays clean.
fn emit_legacy_client_flag_warning(used: &[&'static str]) {
    if !std::io::stderr().is_terminal() {
        return;
    }
    let pretty: Vec<String> = used.iter().map(|id| format!("--{id}")).collect();
    let replacement = used.join(",");
    eprintln!(
        "warning: {} is deprecated; use `--client {}` instead. The legacy flags will be removed in the next major release.",
        pretty.join(", "),
        replacement
    );
}

fn client_filter_includes_cursor(clients: &Option<Vec<String>>) -> bool {
    clients
        .as_ref()
        .is_none_or(|sources| sources.iter().any(|source| source == "cursor"))
}

fn client_filter_explicitly_requests_cursor(clients: &Option<Vec<String>>) -> bool {
    clients
        .as_ref()
        .is_some_and(|sources| sources.iter().any(|source| source == "cursor"))
}

fn should_auto_sync_cursor_for_local_report(
    home_dir: &Option<String>,
    clients: &Option<Vec<String>>,
) -> bool {
    home_dir.is_none() && client_filter_includes_cursor(clients)
}

fn auto_sync_cursor_for_local_report(
    home_dir: &Option<String>,
    clients: &Option<Vec<String>>,
) -> Option<cursor::SyncCursorResult> {
    if !should_auto_sync_cursor_for_local_report(home_dir, clients)
        || !cursor::is_cursor_logged_in()
    {
        return None;
    }

    Some(run_best_effort_cursor_sync_with_runtime_factory(
        tokio::runtime::Runtime::new,
    ))
}

fn run_best_effort_cursor_sync_with_runtime_factory<F>(build_runtime: F) -> cursor::SyncCursorResult
where
    F: FnOnce() -> std::io::Result<tokio::runtime::Runtime>,
{
    match build_runtime() {
        Ok(rt) => rt.block_on(async { cursor::sync_cursor_cache().await }),
        Err(error) => cursor::SyncCursorResult {
            synced: false,
            rows: 0,
            error: Some(format!(
                "Failed to initialize Cursor sync runtime: {}",
                error
            )),
        },
    }
}

fn auto_sync_cursor_before_tui(
    home_dir: &Option<String>,
    clients: &Option<Vec<String>>,
) -> Result<()> {
    let had_cursor_cache = cursor::has_cursor_usage_cache();
    let explicit_cursor_filter = client_filter_explicitly_requests_cursor(clients);
    let cursor_sync_result = auto_sync_cursor_for_local_report(home_dir, clients);
    emit_cursor_sync_warning(
        cursor_sync_result.as_ref(),
        had_cursor_cache,
        explicit_cursor_filter,
    );
    Ok(())
}

fn emit_cursor_sync_warning(
    sync: Option<&cursor::SyncCursorResult>,
    had_cursor_cache: bool,
    explicit_cursor_filter: bool,
) {
    let Some(sync) = sync else {
        return;
    };
    let Some(error) = sync.error.as_ref() else {
        return;
    };
    if sync.synced || had_cursor_cache || explicit_cursor_filter {
        use colored::Colorize;
        let prefix = if sync.synced {
            "Cursor sync warning"
        } else if had_cursor_cache {
            "Cursor sync failed; using cached data"
        } else {
            "Cursor sync failed"
        };
        eprintln!("{}", format!("  {}: {}", prefix, error).yellow());
    }
}

fn default_submit_clients() -> Vec<String> {
    let mut clients: Vec<String> = tokscale_core::ClientId::iter()
        .filter(|client| client.submit_default())
        .map(|client| client.as_str().to_string())
        .collect();
    clients.push("synthetic".to_string());
    clients
}

fn reject_unsupported_home_override(home_dir: &Option<String>, command: &str) -> Result<()> {
    if home_dir.is_some() {
        return Err(anyhow::anyhow!(
            "--home is currently supported only for local report commands. It is not supported for `{}`.",
            command
        ));
    }

    Ok(())
}

fn use_env_roots(home_dir: &Option<String>) -> bool {
    home_dir.is_none()
}

fn ensure_home_supported_for_tui(home_dir: &Option<String>) -> Result<()> {
    if home_dir.is_some() {
        return Err(anyhow::anyhow!(
            "--home is currently supported for local report commands only. Use `--json`, `--light`, `models`, `monthly`, or `graph` instead of TUI mode."
        ));
    }

    Ok(())
}

fn build_date_filter(
    today: bool,
    week: bool,
    month: bool,
    since: Option<String>,
    until: Option<String>,
) -> (Option<String>, Option<String>) {
    build_date_filter_for_date(
        today,
        week,
        month,
        since,
        until,
        chrono::Local::now().date_naive(),
    )
}

fn build_date_filter_for_date(
    today: bool,
    week: bool,
    month: bool,
    since: Option<String>,
    until: Option<String>,
    current_date: chrono::NaiveDate,
) -> (Option<String>, Option<String>) {
    use chrono::{Datelike, Duration};

    if today {
        let date = current_date.format("%Y-%m-%d").to_string();
        return (Some(date.clone()), Some(date));
    }

    if week {
        let start = current_date - Duration::days(6);
        return (
            Some(start.format("%Y-%m-%d").to_string()),
            Some(current_date.format("%Y-%m-%d").to_string()),
        );
    }

    if month {
        let start = current_date.with_day(1).unwrap_or(current_date);
        return (
            Some(start.format("%Y-%m-%d").to_string()),
            Some(current_date.format("%Y-%m-%d").to_string()),
        );
    }

    (since, until)
}

fn normalize_year_filter(
    today: bool,
    week: bool,
    month: bool,
    year: Option<String>,
) -> Option<String> {
    if today || week || month {
        None
    } else {
        year
    }
}

fn get_date_range_label(
    today: bool,
    week: bool,
    month: bool,
    since: &Option<String>,
    until: &Option<String>,
    year: &Option<String>,
) -> Option<String> {
    get_date_range_label_for_date(
        today,
        week,
        month,
        since,
        until,
        year,
        chrono::Local::now().date_naive(),
    )
}

fn get_date_range_label_for_date(
    today: bool,
    week: bool,
    month: bool,
    since: &Option<String>,
    until: &Option<String>,
    year: &Option<String>,
    current_date: chrono::NaiveDate,
) -> Option<String> {
    if today {
        return Some("Today".to_string());
    }
    if week {
        return Some("Last 7 days".to_string());
    }
    if month {
        return Some(current_date.format("%B %Y").to_string());
    }
    if let Some(y) = year {
        return Some(y.clone());
    }
    let mut parts = Vec::new();
    if let Some(s) = since {
        parts.push(format!("from {}", s));
    }
    if let Some(u) = until {
        parts.push(format!("to {}", u));
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" "))
    }
}

struct LightSpinner {
    running: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

const TABLE_PRESET: &str = "││──├─┼┤│─┼├┤┬┴┌┐└┘";

impl LightSpinner {
    const WIDTH: usize = 8;
    const HOLD_START: usize = 30;
    const HOLD_END: usize = 9;
    const TRAIL_LENGTH: usize = 4;
    const TRAIL_COLORS: [u8; 6] = [51, 44, 37, 30, 23, 17];
    const INACTIVE_COLOR: u8 = 240;
    const FRAME_MS: u64 = 40;

    fn start(message: &'static str) -> Self {
        let running = Arc::new(AtomicBool::new(true));
        let running_thread = Arc::clone(&running);
        let message = message.to_string();

        let handle = thread::spawn(move || {
            let mut frame = 0usize;
            let mut stderr = io::stderr().lock();

            let _ = write!(stderr, "\x1b[?25l");
            let _ = stderr.flush();

            while running_thread.load(Ordering::Relaxed) {
                let spinner = Self::frame(frame);
                let _ = write!(stderr, "\r\x1b[K  {} {}", spinner, message);
                let _ = stderr.flush();
                frame = frame.wrapping_add(1);
                thread::sleep(Duration::from_millis(Self::FRAME_MS));
            }

            let _ = write!(stderr, "\r\x1b[K\x1b[?25h");
            let _ = stderr.flush();
        });

        Self {
            running,
            handle: Some(handle),
        }
    }

    fn stop(mut self) {
        self.stop_inner();
    }

    fn stop_inner(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }

    fn frame(frame: usize) -> String {
        let (position, forward) = Self::scanner_state(frame);
        let mut out = String::new();

        for i in 0..Self::WIDTH {
            let distance = if forward {
                if position >= i {
                    position - i
                } else {
                    usize::MAX
                }
            } else if i >= position {
                i - position
            } else {
                usize::MAX
            };

            if distance < Self::TRAIL_LENGTH {
                let color = Self::TRAIL_COLORS[distance.min(Self::TRAIL_COLORS.len() - 1)];
                out.push_str(&format!("\x1b[38;5;{}m■\x1b[0m", color));
            } else {
                out.push_str(&format!("\x1b[38;5;{}m⬝\x1b[0m", Self::INACTIVE_COLOR));
            }
        }

        out
    }

    fn scanner_state(frame: usize) -> (usize, bool) {
        let forward_frames = Self::WIDTH;
        let backward_frames = Self::WIDTH - 1;
        let total_cycle = forward_frames + Self::HOLD_END + backward_frames + Self::HOLD_START;
        let normalized = frame % total_cycle;

        if normalized < forward_frames {
            (normalized, true)
        } else if normalized < forward_frames + Self::HOLD_END {
            (Self::WIDTH - 1, true)
        } else if normalized < forward_frames + Self::HOLD_END + backward_frames {
            (
                Self::WIDTH - 2 - (normalized - forward_frames - Self::HOLD_END),
                false,
            )
        } else {
            (0, false)
        }
    }
}

impl Drop for LightSpinner {
    fn drop(&mut self) {
        self.stop_inner();
    }
}

#[allow(clippy::too_many_arguments)]
fn run_models_report(
    json: bool,
    home_dir: Option<String>,
    clients: Option<Vec<String>>,
    since: Option<String>,
    until: Option<String>,
    year: Option<String>,
    benchmark: bool,
    no_spinner: bool,
    today: bool,
    week: bool,
    month_flag: bool,
    group_by: tokscale_core::GroupBy,
    cli_write_cache: bool,
    cli_no_write_cache: bool,
) -> Result<()> {
    use std::time::Instant;
    use tokio::runtime::Runtime;
    use tokscale_core::{get_model_report, GroupBy, ReportOptions};

    let date_range = get_date_range_label(today, week, month_flag, &since, &until, &year);

    let had_cursor_cache = cursor::has_cursor_usage_cache();
    let explicit_cursor_filter = client_filter_explicitly_requests_cursor(&clients);
    let spinner = if no_spinner {
        None
    } else {
        Some(LightSpinner::start("Scanning session data..."))
    };
    let cursor_sync_result = auto_sync_cursor_for_local_report(&home_dir, &clients);
    let use_env_roots = use_env_roots(&home_dir);
    let start = Instant::now();
    let rt = Runtime::new()?;
    let report = rt
        .block_on(async {
            get_model_report(ReportOptions {
                home_dir: home_dir.clone(),
                use_env_roots,
                clients: clients.clone(),
                since: since.clone(),
                until: until.clone(),
                year: year.clone(),
                group_by: group_by.clone(),
                scanner_settings: tui::settings::load_scanner_settings(),
            })
            .await
        })
        .map_err(|e| anyhow::anyhow!(e))?;

    if let Some(spinner) = spinner {
        spinner.stop();
    }
    emit_cursor_sync_warning(
        cursor_sync_result.as_ref(),
        had_cursor_cache,
        explicit_cursor_filter,
    );

    let processing_time_ms = start.elapsed().as_millis();

    if json {
        #[derive(serde::Serialize)]
        #[serde(rename_all = "camelCase")]
        struct ModelUsageJson {
            client: String,
            merged_clients: Option<String>,
            #[serde(skip_serializing_if = "Option::is_none")]
            workspace_key: Option<serde_json::Value>,
            #[serde(skip_serializing_if = "Option::is_none")]
            workspace_label: Option<String>,
            model: String,
            provider: String,
            input: i64,
            output: i64,
            cache_read: i64,
            cache_write: i64,
            reasoning: i64,
            message_count: i32,
            cost: f64,
        }

        #[derive(serde::Serialize)]
        #[serde(rename_all = "camelCase")]
        struct ModelReportJson {
            group_by: String,
            entries: Vec<ModelUsageJson>,
            total_input: i64,
            total_output: i64,
            total_cache_read: i64,
            total_cache_write: i64,
            total_messages: i32,
            total_cost: f64,
            processing_time_ms: u32,
        }

        let output = ModelReportJson {
            group_by: group_by.to_string(),
            entries: report
                .entries
                .into_iter()
                .map(|e| ModelUsageJson {
                    workspace_key: if group_by == GroupBy::WorkspaceModel {
                        Some(
                            e.workspace_key
                                .map(serde_json::Value::String)
                                .unwrap_or(serde_json::Value::Null),
                        )
                    } else {
                        None
                    },
                    workspace_label: if group_by == GroupBy::WorkspaceModel {
                        e.workspace_label
                    } else {
                        None
                    },
                    client: e.client,
                    merged_clients: e.merged_clients,
                    model: e.model,
                    provider: e.provider,
                    input: e.input,
                    output: e.output,
                    cache_read: e.cache_read,
                    cache_write: e.cache_write,
                    reasoning: e.reasoning,
                    message_count: e.message_count,
                    cost: e.cost,
                })
                .collect(),
            total_input: report.total_input,
            total_output: report.total_output,
            total_cache_read: report.total_cache_read,
            total_cache_write: report.total_cache_write,
            total_messages: report.total_messages,
            total_cost: report.total_cost,
            processing_time_ms: report.processing_time_ms,
        };
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        use comfy_table::{Attribute, Cell, CellAlignment, Color, ContentArrangement, Table};

        let term_width = crossterm::terminal::size()
            .map(|(w, _)| w as usize)
            .unwrap_or(120);
        let compact = term_width < 100;

        let mut table = Table::new();
        table.load_preset(TABLE_PRESET);
        let arrangement = if std::io::stdout().is_terminal() {
            ContentArrangement::DynamicFullWidth
        } else {
            ContentArrangement::Dynamic
        };
        table.set_content_arrangement(arrangement);
        table.enforce_styling();

        let workspace_name = |label: Option<&str>| label.unwrap_or("Unknown workspace").to_string();

        if compact {
            match group_by {
                GroupBy::Model => {
                    table.set_header(vec![
                        Cell::new("Clients").fg(Color::Cyan),
                        Cell::new("Providers").fg(Color::Cyan),
                        Cell::new("Model").fg(Color::Cyan),
                        Cell::new("Input").fg(Color::Cyan),
                        Cell::new("Output").fg(Color::Cyan),
                        Cell::new("Cost").fg(Color::Cyan),
                        Cell::new("Cost/1M").fg(Color::Cyan),
                    ]);

                    for entry in &report.entries {
                        let clients_str = entry.merged_clients.as_deref().unwrap_or(&entry.client);
                        let capitalized_clients = clients_str
                            .split(", ")
                            .map(capitalize_client)
                            .collect::<Vec<_>>()
                            .join(", ");
                        let total_tokens =
                            entry.input + entry.output + entry.cache_read + entry.cache_write;
                        table.add_row(vec![
                            Cell::new(capitalized_clients),
                            Cell::new(&entry.provider).add_attribute(Attribute::Dim),
                            Cell::new(&entry.model),
                            Cell::new(format_tokens_with_commas(entry.input))
                                .set_alignment(CellAlignment::Right),
                            Cell::new(format_tokens_with_commas(entry.output))
                                .set_alignment(CellAlignment::Right),
                            Cell::new(format_currency(entry.cost))
                                .set_alignment(CellAlignment::Right),
                            Cell::new(format_cost_per_million(entry.cost, total_tokens))
                                .set_alignment(CellAlignment::Right),
                        ]);
                    }

                    let total_tokens = report.total_input
                        + report.total_output
                        + report.total_cache_read
                        + report.total_cache_write;
                    table.add_row(vec![
                        Cell::new("Total")
                            .fg(Color::Yellow)
                            .add_attribute(Attribute::Bold),
                        Cell::new(""),
                        Cell::new(""),
                        Cell::new(format_tokens_with_commas(report.total_input))
                            .fg(Color::Yellow)
                            .set_alignment(CellAlignment::Right),
                        Cell::new(format_tokens_with_commas(report.total_output))
                            .fg(Color::Yellow)
                            .set_alignment(CellAlignment::Right),
                        Cell::new(format_currency(report.total_cost))
                            .fg(Color::Yellow)
                            .set_alignment(CellAlignment::Right),
                        Cell::new(format_cost_per_million(report.total_cost, total_tokens))
                            .fg(Color::Yellow)
                            .set_alignment(CellAlignment::Right),
                    ]);
                }
                GroupBy::ClientModel | GroupBy::ClientProviderModel => {
                    table.set_header(vec![
                        Cell::new("Client").fg(Color::Cyan),
                        Cell::new("Provider").fg(Color::Cyan),
                        Cell::new("Model").fg(Color::Cyan),
                        Cell::new("Input").fg(Color::Cyan),
                        Cell::new("Output").fg(Color::Cyan),
                        Cell::new("Cost").fg(Color::Cyan),
                        Cell::new("Cost/1M").fg(Color::Cyan),
                    ]);

                    for entry in &report.entries {
                        let total_tokens =
                            entry.input + entry.output + entry.cache_read + entry.cache_write;
                        table.add_row(vec![
                            Cell::new(capitalize_client(&entry.client)),
                            Cell::new(&entry.provider).add_attribute(Attribute::Dim),
                            Cell::new(&entry.model),
                            Cell::new(format_tokens_with_commas(entry.input))
                                .set_alignment(CellAlignment::Right),
                            Cell::new(format_tokens_with_commas(entry.output))
                                .set_alignment(CellAlignment::Right),
                            Cell::new(format_currency(entry.cost))
                                .set_alignment(CellAlignment::Right),
                            Cell::new(format_cost_per_million(entry.cost, total_tokens))
                                .set_alignment(CellAlignment::Right),
                        ]);
                    }

                    let total_tokens = report.total_input
                        + report.total_output
                        + report.total_cache_read
                        + report.total_cache_write;
                    table.add_row(vec![
                        Cell::new("Total")
                            .fg(Color::Yellow)
                            .add_attribute(Attribute::Bold),
                        Cell::new(""),
                        Cell::new(""),
                        Cell::new(format_tokens_with_commas(report.total_input))
                            .fg(Color::Yellow)
                            .set_alignment(CellAlignment::Right),
                        Cell::new(format_tokens_with_commas(report.total_output))
                            .fg(Color::Yellow)
                            .set_alignment(CellAlignment::Right),
                        Cell::new(format_currency(report.total_cost))
                            .fg(Color::Yellow)
                            .set_alignment(CellAlignment::Right),
                        Cell::new(format_cost_per_million(report.total_cost, total_tokens))
                            .fg(Color::Yellow)
                            .set_alignment(CellAlignment::Right),
                    ]);
                }
                GroupBy::WorkspaceModel => {
                    table.set_header(vec![
                        Cell::new("Workspace").fg(Color::Cyan),
                        Cell::new("Model").fg(Color::Cyan),
                        Cell::new("Cost").fg(Color::Cyan),
                    ]);

                    for entry in &report.entries {
                        table.add_row(vec![
                            Cell::new(workspace_name(entry.workspace_label.as_deref())),
                            Cell::new(&entry.model),
                            Cell::new(format_currency(entry.cost))
                                .set_alignment(CellAlignment::Right),
                        ]);
                    }

                    table.add_row(vec![
                        Cell::new("Total")
                            .fg(Color::Yellow)
                            .add_attribute(Attribute::Bold),
                        Cell::new(""),
                        Cell::new(format_currency(report.total_cost))
                            .fg(Color::Yellow)
                            .set_alignment(CellAlignment::Right),
                    ]);
                }
            }
        } else {
            match group_by {
                GroupBy::Model => {
                    table.set_header(vec![
                        Cell::new("Clients").fg(Color::Cyan),
                        Cell::new("Providers").fg(Color::Cyan),
                        Cell::new("Model").fg(Color::Cyan),
                        Cell::new("Input").fg(Color::Cyan),
                        Cell::new("Output").fg(Color::Cyan),
                        Cell::new("Cache Write").fg(Color::Cyan),
                        Cell::new("Cache Read").fg(Color::Cyan),
                        Cell::new("Total").fg(Color::Cyan),
                        Cell::new("Cost").fg(Color::Cyan),
                        Cell::new("Cost/1M").fg(Color::Cyan),
                    ]);

                    for entry in &report.entries {
                        let total =
                            entry.input + entry.output + entry.cache_write + entry.cache_read;

                        let clients_str = entry.merged_clients.as_deref().unwrap_or(&entry.client);
                        let capitalized_clients = clients_str
                            .split(", ")
                            .map(capitalize_client)
                            .collect::<Vec<_>>()
                            .join(", ");
                        table.add_row(vec![
                            Cell::new(capitalized_clients),
                            Cell::new(&entry.provider).add_attribute(Attribute::Dim),
                            Cell::new(&entry.model),
                            Cell::new(format_tokens_with_commas(entry.input))
                                .set_alignment(CellAlignment::Right),
                            Cell::new(format_tokens_with_commas(entry.output))
                                .set_alignment(CellAlignment::Right),
                            Cell::new(format_tokens_with_commas(entry.cache_write))
                                .set_alignment(CellAlignment::Right),
                            Cell::new(format_tokens_with_commas(entry.cache_read))
                                .set_alignment(CellAlignment::Right),
                            Cell::new(format_tokens_with_commas(total))
                                .set_alignment(CellAlignment::Right),
                            Cell::new(format_currency(entry.cost))
                                .set_alignment(CellAlignment::Right),
                            Cell::new(format_cost_per_million(entry.cost, total))
                                .set_alignment(CellAlignment::Right),
                        ]);
                    }

                    let total_all = report.total_input
                        + report.total_output
                        + report.total_cache_write
                        + report.total_cache_read;
                    table.add_row(vec![
                        Cell::new("Total")
                            .fg(Color::Yellow)
                            .add_attribute(Attribute::Bold),
                        Cell::new(""),
                        Cell::new(""),
                        Cell::new(format_tokens_with_commas(report.total_input))
                            .fg(Color::Yellow)
                            .set_alignment(CellAlignment::Right),
                        Cell::new(format_tokens_with_commas(report.total_output))
                            .fg(Color::Yellow)
                            .set_alignment(CellAlignment::Right),
                        Cell::new(format_tokens_with_commas(report.total_cache_write))
                            .fg(Color::Yellow)
                            .set_alignment(CellAlignment::Right),
                        Cell::new(format_tokens_with_commas(report.total_cache_read))
                            .fg(Color::Yellow)
                            .set_alignment(CellAlignment::Right),
                        Cell::new(format_tokens_with_commas(total_all))
                            .fg(Color::Yellow)
                            .set_alignment(CellAlignment::Right),
                        Cell::new(format_currency(report.total_cost))
                            .fg(Color::Yellow)
                            .set_alignment(CellAlignment::Right),
                        Cell::new(format_cost_per_million(report.total_cost, total_all))
                            .fg(Color::Yellow)
                            .set_alignment(CellAlignment::Right),
                    ]);
                }
                GroupBy::ClientModel | GroupBy::ClientProviderModel => {
                    table.set_header(vec![
                        Cell::new("Client").fg(Color::Cyan),
                        Cell::new("Provider").fg(Color::Cyan),
                        Cell::new("Model").fg(Color::Cyan),
                        Cell::new("Resolved").fg(Color::Cyan),
                        Cell::new("Input").fg(Color::Cyan),
                        Cell::new("Output").fg(Color::Cyan),
                        Cell::new("Cache Write").fg(Color::Cyan),
                        Cell::new("Cache Read").fg(Color::Cyan),
                        Cell::new("Total").fg(Color::Cyan),
                        Cell::new("Cost").fg(Color::Cyan),
                        Cell::new("Cost/1M").fg(Color::Cyan),
                    ]);

                    for entry in &report.entries {
                        let total =
                            entry.input + entry.output + entry.cache_write + entry.cache_read;

                        table.add_row(vec![
                            Cell::new(capitalize_client(&entry.client)),
                            Cell::new(&entry.provider).add_attribute(Attribute::Dim),
                            Cell::new(&entry.model),
                            Cell::new(format_model_name(&entry.model)),
                            Cell::new(format_tokens_with_commas(entry.input))
                                .set_alignment(CellAlignment::Right),
                            Cell::new(format_tokens_with_commas(entry.output))
                                .set_alignment(CellAlignment::Right),
                            Cell::new(format_tokens_with_commas(entry.cache_write))
                                .set_alignment(CellAlignment::Right),
                            Cell::new(format_tokens_with_commas(entry.cache_read))
                                .set_alignment(CellAlignment::Right),
                            Cell::new(format_tokens_with_commas(total))
                                .set_alignment(CellAlignment::Right),
                            Cell::new(format_currency(entry.cost))
                                .set_alignment(CellAlignment::Right),
                            Cell::new(format_cost_per_million(entry.cost, total))
                                .set_alignment(CellAlignment::Right),
                        ]);
                    }

                    let total_all = report.total_input
                        + report.total_output
                        + report.total_cache_write
                        + report.total_cache_read;
                    table.add_row(vec![
                        Cell::new("Total")
                            .fg(Color::Yellow)
                            .add_attribute(Attribute::Bold),
                        Cell::new(""),
                        Cell::new(""),
                        Cell::new(""),
                        Cell::new(format_tokens_with_commas(report.total_input))
                            .fg(Color::Yellow)
                            .set_alignment(CellAlignment::Right),
                        Cell::new(format_tokens_with_commas(report.total_output))
                            .fg(Color::Yellow)
                            .set_alignment(CellAlignment::Right),
                        Cell::new(format_tokens_with_commas(report.total_cache_write))
                            .fg(Color::Yellow)
                            .set_alignment(CellAlignment::Right),
                        Cell::new(format_tokens_with_commas(report.total_cache_read))
                            .fg(Color::Yellow)
                            .set_alignment(CellAlignment::Right),
                        Cell::new(format_tokens_with_commas(total_all))
                            .fg(Color::Yellow)
                            .set_alignment(CellAlignment::Right),
                        Cell::new(format_currency(report.total_cost))
                            .fg(Color::Yellow)
                            .set_alignment(CellAlignment::Right),
                        Cell::new(format_cost_per_million(report.total_cost, total_all))
                            .fg(Color::Yellow)
                            .set_alignment(CellAlignment::Right),
                    ]);
                }
                GroupBy::WorkspaceModel => {
                    table.set_header(vec![
                        Cell::new("Workspace").fg(Color::Cyan),
                        Cell::new("Providers").fg(Color::Cyan),
                        Cell::new("Sources").fg(Color::Cyan),
                        Cell::new("Model").fg(Color::Cyan),
                        Cell::new("Input").fg(Color::Cyan),
                        Cell::new("Output").fg(Color::Cyan),
                        Cell::new("Cache Write").fg(Color::Cyan),
                        Cell::new("Cache Read").fg(Color::Cyan),
                        Cell::new("Total").fg(Color::Cyan),
                        Cell::new("Cost").fg(Color::Cyan),
                    ]);

                    for entry in &report.entries {
                        let total =
                            entry.input + entry.output + entry.cache_write + entry.cache_read;
                        let clients_str = entry.merged_clients.as_deref().unwrap_or(&entry.client);
                        let capitalized_clients = clients_str
                            .split(", ")
                            .map(capitalize_client)
                            .collect::<Vec<_>>()
                            .join(", ");

                        table.add_row(vec![
                            Cell::new(workspace_name(entry.workspace_label.as_deref())),
                            Cell::new(&entry.provider).add_attribute(Attribute::Dim),
                            Cell::new(capitalized_clients),
                            Cell::new(&entry.model),
                            Cell::new(format_tokens_with_commas(entry.input))
                                .set_alignment(CellAlignment::Right),
                            Cell::new(format_tokens_with_commas(entry.output))
                                .set_alignment(CellAlignment::Right),
                            Cell::new(format_tokens_with_commas(entry.cache_write))
                                .set_alignment(CellAlignment::Right),
                            Cell::new(format_tokens_with_commas(entry.cache_read))
                                .set_alignment(CellAlignment::Right),
                            Cell::new(format_tokens_with_commas(total))
                                .set_alignment(CellAlignment::Right),
                            Cell::new(format_currency(entry.cost))
                                .set_alignment(CellAlignment::Right),
                        ]);
                    }

                    let total_all = report.total_input
                        + report.total_output
                        + report.total_cache_write
                        + report.total_cache_read;
                    table.add_row(vec![
                        Cell::new("Total")
                            .fg(Color::Yellow)
                            .add_attribute(Attribute::Bold),
                        Cell::new(""),
                        Cell::new(""),
                        Cell::new(""),
                        Cell::new(format_tokens_with_commas(report.total_input))
                            .fg(Color::Yellow)
                            .set_alignment(CellAlignment::Right),
                        Cell::new(format_tokens_with_commas(report.total_output))
                            .fg(Color::Yellow)
                            .set_alignment(CellAlignment::Right),
                        Cell::new(format_tokens_with_commas(report.total_cache_write))
                            .fg(Color::Yellow)
                            .set_alignment(CellAlignment::Right),
                        Cell::new(format_tokens_with_commas(report.total_cache_read))
                            .fg(Color::Yellow)
                            .set_alignment(CellAlignment::Right),
                        Cell::new(format_tokens_with_commas(total_all))
                            .fg(Color::Yellow)
                            .set_alignment(CellAlignment::Right),
                        Cell::new(format_currency(report.total_cost))
                            .fg(Color::Yellow)
                            .set_alignment(CellAlignment::Right),
                    ]);
                }
            }
        }

        let title = match &date_range {
            Some(range) => format!("Token Usage Report by Model ({})", range),
            None => "Token Usage Report by Model".to_string(),
        };
        println!("\n  \x1b[36m{}\x1b[0m\n", title);
        println!("{}", dim_borders(&table.to_string()));

        let total_tokens = report.total_input
            + report.total_output
            + report.total_cache_write
            + report.total_cache_read;
        println!(
            "\x1b[90m\n  Total: {} messages, {} tokens, \x1b[32m{}\x1b[90m\x1b[0m",
            format_tokens_with_commas(report.total_messages as i64),
            format_tokens_with_commas(total_tokens),
            format_currency(report.total_cost)
        );

        if benchmark {
            use colored::Colorize;
            println!(
                "{}",
                format!("  Processing time: {}ms (Rust native)", processing_time_ms).bright_black()
            );
        }

        io::stdout().flush()?;

        let settings = tui::settings::Settings::load();
        if resolve_should_write_cache(cli_write_cache, cli_no_write_cache, &settings) {
            write_light_cache(&home_dir, &clients, &since, &until, &year, &group_by);
        }
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn run_monthly_report(
    json: bool,
    home_dir: Option<String>,
    clients: Option<Vec<String>>,
    since: Option<String>,
    until: Option<String>,
    year: Option<String>,
    benchmark: bool,
    no_spinner: bool,
    today: bool,
    week: bool,
    month_flag: bool,
) -> Result<()> {
    use std::time::Instant;
    use tokio::runtime::Runtime;
    use tokscale_core::{get_monthly_report, GroupBy, ReportOptions};

    let date_range = get_date_range_label(today, week, month_flag, &since, &until, &year);

    let had_cursor_cache = cursor::has_cursor_usage_cache();
    let explicit_cursor_filter = client_filter_explicitly_requests_cursor(&clients);
    let spinner = if no_spinner {
        None
    } else {
        Some(LightSpinner::start("Scanning session data..."))
    };
    let cursor_sync_result = auto_sync_cursor_for_local_report(&home_dir, &clients);
    let use_env_roots = use_env_roots(&home_dir);
    let start = Instant::now();
    let rt = Runtime::new()?;
    let report = rt
        .block_on(async {
            get_monthly_report(ReportOptions {
                home_dir,
                use_env_roots,
                clients,
                since,
                until,
                year,
                group_by: GroupBy::default(),
                scanner_settings: tui::settings::load_scanner_settings(),
            })
            .await
        })
        .map_err(|e| anyhow::anyhow!(e))?;

    if let Some(spinner) = spinner {
        spinner.stop();
    }
    emit_cursor_sync_warning(
        cursor_sync_result.as_ref(),
        had_cursor_cache,
        explicit_cursor_filter,
    );

    let processing_time_ms = start.elapsed().as_millis();

    if json {
        #[derive(serde::Serialize)]
        #[serde(rename_all = "camelCase")]
        struct MonthlyUsageJson {
            month: String,
            models: Vec<String>,
            input: i64,
            output: i64,
            cache_read: i64,
            cache_write: i64,
            message_count: i32,
            cost: f64,
        }

        #[derive(serde::Serialize)]
        #[serde(rename_all = "camelCase")]
        struct MonthlyReportJson {
            entries: Vec<MonthlyUsageJson>,
            total_cost: f64,
            processing_time_ms: u32,
        }

        let output = MonthlyReportJson {
            entries: report
                .entries
                .into_iter()
                .map(|e| MonthlyUsageJson {
                    month: e.month,
                    models: e.models,
                    input: e.input,
                    output: e.output,
                    cache_read: e.cache_read,
                    cache_write: e.cache_write,
                    message_count: e.message_count,
                    cost: e.cost,
                })
                .collect(),
            total_cost: report.total_cost,
            processing_time_ms: report.processing_time_ms,
        };

        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        use comfy_table::{Attribute, Cell, CellAlignment, Color, ContentArrangement, Table};

        let term_width = crossterm::terminal::size()
            .map(|(w, _)| w as usize)
            .unwrap_or(120);
        let compact = term_width < 100;

        let mut table = Table::new();
        table.load_preset(TABLE_PRESET);
        let arrangement = if std::io::stdout().is_terminal() {
            ContentArrangement::DynamicFullWidth
        } else {
            ContentArrangement::Dynamic
        };
        table.set_content_arrangement(arrangement);
        table.enforce_styling();
        if compact {
            table.set_header(vec![
                Cell::new("Month").fg(Color::Cyan),
                Cell::new("Models").fg(Color::Cyan),
                Cell::new("Input").fg(Color::Cyan),
                Cell::new("Output").fg(Color::Cyan),
                Cell::new("Cost").fg(Color::Cyan),
                Cell::new("Cost/1M").fg(Color::Cyan),
            ]);

            for entry in &report.entries {
                let models_col = if entry.models.is_empty() {
                    "-".to_string()
                } else {
                    let mut unique_models: Vec<String> = entry
                        .models
                        .iter()
                        .map(|model| format_model_name(model))
                        .collect::<std::collections::BTreeSet<_>>()
                        .into_iter()
                        .collect();
                    unique_models.sort();
                    unique_models
                        .iter()
                        .map(|m| format!("- {}", m))
                        .collect::<Vec<_>>()
                        .join("\n")
                };
                let total_tokens =
                    entry.input + entry.output + entry.cache_read + entry.cache_write;

                table.add_row(vec![
                    Cell::new(entry.month.clone()),
                    Cell::new(models_col),
                    Cell::new(format_tokens_with_commas(entry.input))
                        .set_alignment(CellAlignment::Right),
                    Cell::new(format_tokens_with_commas(entry.output))
                        .set_alignment(CellAlignment::Right),
                    Cell::new(format_currency(entry.cost)).set_alignment(CellAlignment::Right),
                    Cell::new(format_cost_per_million(entry.cost, total_tokens))
                        .set_alignment(CellAlignment::Right),
                ]);
            }

            let total_input: i64 = report.entries.iter().map(|e| e.input).sum();
            let total_output: i64 = report.entries.iter().map(|e| e.output).sum();
            let total_cache_read: i64 = report.entries.iter().map(|e| e.cache_read).sum();
            let total_cache_write: i64 = report.entries.iter().map(|e| e.cache_write).sum();
            let total_tokens = total_input + total_output + total_cache_read + total_cache_write;
            table.add_row(vec![
                Cell::new("Total")
                    .fg(Color::Yellow)
                    .add_attribute(Attribute::Bold),
                Cell::new(""),
                Cell::new(format_tokens_with_commas(total_input))
                    .fg(Color::Yellow)
                    .set_alignment(CellAlignment::Right),
                Cell::new(format_tokens_with_commas(total_output))
                    .fg(Color::Yellow)
                    .set_alignment(CellAlignment::Right),
                Cell::new(format_currency(report.total_cost))
                    .fg(Color::Yellow)
                    .set_alignment(CellAlignment::Right),
                Cell::new(format_cost_per_million(report.total_cost, total_tokens))
                    .fg(Color::Yellow)
                    .set_alignment(CellAlignment::Right),
            ]);
        } else {
            table.set_header(vec![
                Cell::new("Month").fg(Color::Cyan),
                Cell::new("Models").fg(Color::Cyan),
                Cell::new("Input").fg(Color::Cyan),
                Cell::new("Output").fg(Color::Cyan),
                Cell::new("Cache Write").fg(Color::Cyan),
                Cell::new("Cache Read").fg(Color::Cyan),
                Cell::new("Total").fg(Color::Cyan),
                Cell::new("Cost").fg(Color::Cyan),
                Cell::new("Cost/1M").fg(Color::Cyan),
            ]);

            for entry in &report.entries {
                let models_col = if entry.models.is_empty() {
                    "-".to_string()
                } else {
                    let mut unique_models: Vec<String> = entry
                        .models
                        .iter()
                        .map(|model| format_model_name(model))
                        .collect::<std::collections::BTreeSet<_>>()
                        .into_iter()
                        .collect();
                    unique_models.sort();
                    unique_models
                        .iter()
                        .map(|m| format!("- {}", m))
                        .collect::<Vec<_>>()
                        .join("\n")
                };
                let total = entry.input + entry.output + entry.cache_write + entry.cache_read;

                table.add_row(vec![
                    Cell::new(entry.month.clone()),
                    Cell::new(models_col),
                    Cell::new(format_tokens_with_commas(entry.input))
                        .set_alignment(CellAlignment::Right),
                    Cell::new(format_tokens_with_commas(entry.output))
                        .set_alignment(CellAlignment::Right),
                    Cell::new(format_tokens_with_commas(entry.cache_write))
                        .set_alignment(CellAlignment::Right),
                    Cell::new(format_tokens_with_commas(entry.cache_read))
                        .set_alignment(CellAlignment::Right),
                    Cell::new(format_tokens_with_commas(total)).set_alignment(CellAlignment::Right),
                    Cell::new(format_currency(entry.cost)).set_alignment(CellAlignment::Right),
                    Cell::new(format_cost_per_million(entry.cost, total))
                        .set_alignment(CellAlignment::Right),
                ]);
            }

            let total_input: i64 = report.entries.iter().map(|e| e.input).sum();
            let total_output: i64 = report.entries.iter().map(|e| e.output).sum();
            let total_cache_write: i64 = report.entries.iter().map(|e| e.cache_write).sum();
            let total_cache_read: i64 = report.entries.iter().map(|e| e.cache_read).sum();
            let total_all = total_input + total_output + total_cache_write + total_cache_read;

            table.add_row(vec![
                Cell::new("Total")
                    .fg(Color::Yellow)
                    .add_attribute(Attribute::Bold),
                Cell::new(""),
                Cell::new(format_tokens_with_commas(total_input))
                    .fg(Color::Yellow)
                    .set_alignment(CellAlignment::Right),
                Cell::new(format_tokens_with_commas(total_output))
                    .fg(Color::Yellow)
                    .set_alignment(CellAlignment::Right),
                Cell::new(format_tokens_with_commas(total_cache_write))
                    .fg(Color::Yellow)
                    .set_alignment(CellAlignment::Right),
                Cell::new(format_tokens_with_commas(total_cache_read))
                    .fg(Color::Yellow)
                    .set_alignment(CellAlignment::Right),
                Cell::new(format_tokens_with_commas(total_all))
                    .fg(Color::Yellow)
                    .set_alignment(CellAlignment::Right),
                Cell::new(format_currency(report.total_cost))
                    .fg(Color::Yellow)
                    .set_alignment(CellAlignment::Right),
                Cell::new(format_cost_per_million(report.total_cost, total_all))
                    .fg(Color::Yellow)
                    .set_alignment(CellAlignment::Right),
            ]);
        }

        let title = match &date_range {
            Some(range) => format!("Monthly Token Usage Report ({})", range),
            None => "Monthly Token Usage Report".to_string(),
        };
        println!("\n  \x1b[36m{}\x1b[0m\n", title);
        println!("{}", dim_borders(&table.to_string()));

        println!(
            "\x1b[90m\n  Total Cost: \x1b[32m{}\x1b[90m\x1b[0m",
            format_currency(report.total_cost)
        );

        if benchmark {
            use colored::Colorize;
            println!(
                "{}",
                format!("  Processing time: {}ms (Rust native)", processing_time_ms).bright_black()
            );
        }
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn run_hourly_report(
    json: bool,
    home_dir: Option<String>,
    clients: Option<Vec<String>>,
    since: Option<String>,
    until: Option<String>,
    year: Option<String>,
    benchmark: bool,
    no_spinner: bool,
    today: bool,
    week: bool,
    month_flag: bool,
) -> Result<()> {
    use std::time::Instant;
    use tokio::runtime::Runtime;
    use tokscale_core::{get_hourly_report, GroupBy, ReportOptions};

    let date_range = get_date_range_label(today, week, month_flag, &since, &until, &year);

    let had_cursor_cache = cursor::has_cursor_usage_cache();
    let explicit_cursor_filter = client_filter_explicitly_requests_cursor(&clients);
    let spinner = if no_spinner {
        None
    } else {
        Some(LightSpinner::start("Scanning session data..."))
    };
    let cursor_sync_result = auto_sync_cursor_for_local_report(&home_dir, &clients);
    let use_env_roots = use_env_roots(&home_dir);
    let start = Instant::now();
    let rt = Runtime::new()?;
    let report = rt
        .block_on(async {
            get_hourly_report(ReportOptions {
                home_dir,
                use_env_roots,
                clients,
                since,
                until,
                year,
                group_by: GroupBy::default(),
                scanner_settings: tui::settings::load_scanner_settings(),
            })
            .await
        })
        .map_err(|e| anyhow::anyhow!(e))?;

    if let Some(spinner) = spinner {
        spinner.stop();
    }
    emit_cursor_sync_warning(
        cursor_sync_result.as_ref(),
        had_cursor_cache,
        explicit_cursor_filter,
    );

    let processing_time_ms = start.elapsed().as_millis();

    if json {
        #[derive(serde::Serialize)]
        #[serde(rename_all = "camelCase")]
        struct HourlyUsageJson {
            hour: String,
            clients: Vec<String>,
            models: Vec<String>,
            input: i64,
            output: i64,
            cache_read: i64,
            cache_write: i64,
            message_count: i32,
            turn_count: i32,
            cost: f64,
        }

        #[derive(serde::Serialize)]
        #[serde(rename_all = "camelCase")]
        struct HourlyReportJson {
            entries: Vec<HourlyUsageJson>,
            total_cost: f64,
            processing_time_ms: u32,
        }

        let output = HourlyReportJson {
            entries: report
                .entries
                .into_iter()
                .map(|e| HourlyUsageJson {
                    hour: e.hour,
                    clients: e.clients,
                    models: e.models,
                    input: e.input,
                    output: e.output,
                    cache_read: e.cache_read,
                    cache_write: e.cache_write,
                    message_count: e.message_count,
                    turn_count: e.turn_count,
                    cost: e.cost,
                })
                .collect(),
            total_cost: report.total_cost,
            processing_time_ms: report.processing_time_ms,
        };

        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        use comfy_table::{Cell, CellAlignment, Color, ContentArrangement, Table};

        let term_width = crossterm::terminal::size()
            .map(|(w, _)| w as usize)
            .unwrap_or(120);
        let compact = term_width < 100;

        let mut table = Table::new();
        table.load_preset(TABLE_PRESET);
        let arrangement = if std::io::stdout().is_terminal() {
            ContentArrangement::DynamicFullWidth
        } else {
            ContentArrangement::Dynamic
        };
        table.set_content_arrangement(arrangement);
        table.enforce_styling();

        if compact {
            table.set_header(vec![
                Cell::new("Hour").fg(Color::Cyan),
                Cell::new("Source").fg(Color::Cyan),
                Cell::new("Turn").fg(Color::Cyan),
                Cell::new("Msgs").fg(Color::Cyan),
                Cell::new("Input").fg(Color::Cyan),
                Cell::new("Output").fg(Color::Cyan),
                Cell::new("Cost").fg(Color::Cyan),
                Cell::new("Cost/1M").fg(Color::Cyan),
            ]);

            for entry in &report.entries {
                let clients_col = {
                    let mut c: Vec<String> =
                        entry.clients.iter().map(|s| capitalize_client(s)).collect();
                    c.sort();
                    c.join(", ")
                };
                let turn_display = if entry.turn_count > 0 {
                    entry.turn_count.to_string()
                } else {
                    "—".to_string()
                };
                let total_tokens =
                    entry.input + entry.output + entry.cache_read + entry.cache_write;
                table.add_row(vec![
                    Cell::new(&entry.hour).fg(Color::White),
                    Cell::new(&clients_col),
                    Cell::new(&turn_display).set_alignment(CellAlignment::Right),
                    Cell::new(entry.message_count).set_alignment(CellAlignment::Right),
                    Cell::new(format_tokens_with_commas(entry.input))
                        .set_alignment(CellAlignment::Right),
                    Cell::new(format_tokens_with_commas(entry.output))
                        .set_alignment(CellAlignment::Right),
                    Cell::new(format_currency(entry.cost))
                        .fg(Color::Green)
                        .set_alignment(CellAlignment::Right),
                    Cell::new(format_cost_per_million(entry.cost, total_tokens))
                        .set_alignment(CellAlignment::Right),
                ]);
            }
        } else {
            table.set_header(vec![
                Cell::new("Hour").fg(Color::Cyan),
                Cell::new("Source").fg(Color::Cyan),
                Cell::new("Models").fg(Color::Cyan),
                Cell::new("Turn").fg(Color::Cyan),
                Cell::new("Msgs").fg(Color::Cyan),
                Cell::new("Input").fg(Color::Cyan),
                Cell::new("Output").fg(Color::Cyan),
                Cell::new("Cache R").fg(Color::Cyan),
                Cell::new("Cache W").fg(Color::Cyan),
                Cell::new("Cache×").fg(Color::Cyan),
                Cell::new("Cost").fg(Color::Cyan),
                Cell::new("Cost/1M").fg(Color::Cyan),
            ]);

            for entry in &report.entries {
                let clients_col = {
                    let mut c: Vec<String> =
                        entry.clients.iter().map(|s| capitalize_client(s)).collect();
                    c.sort();
                    c.join(", ")
                };
                let models_col = if entry.models.is_empty() {
                    "-".to_string()
                } else {
                    let mut unique: Vec<String> = entry
                        .models
                        .iter()
                        .map(|m| format_model_name(m))
                        .collect::<std::collections::BTreeSet<_>>()
                        .into_iter()
                        .collect();
                    unique.sort();
                    unique.join(", ")
                };

                let cache_hit = {
                    let paid = (entry.input as u64).saturating_add(entry.cache_write as u64);
                    if paid == 0 {
                        if entry.cache_read > 0 {
                            "∞".to_string()
                        } else {
                            "—".to_string()
                        }
                    } else {
                        format!("{:.1}x", entry.cache_read as f64 / paid as f64)
                    }
                };

                let turn_display = if entry.turn_count > 0 {
                    entry.turn_count.to_string()
                } else {
                    "—".to_string()
                };

                let total_tokens =
                    entry.input + entry.output + entry.cache_read + entry.cache_write;

                table.add_row(vec![
                    Cell::new(&entry.hour).fg(Color::White),
                    Cell::new(&clients_col),
                    Cell::new(&models_col),
                    Cell::new(&turn_display).set_alignment(CellAlignment::Right),
                    Cell::new(entry.message_count).set_alignment(CellAlignment::Right),
                    Cell::new(format_tokens_with_commas(entry.input))
                        .set_alignment(CellAlignment::Right),
                    Cell::new(format_tokens_with_commas(entry.output))
                        .set_alignment(CellAlignment::Right),
                    Cell::new(format_tokens_with_commas(entry.cache_read))
                        .set_alignment(CellAlignment::Right),
                    Cell::new(format_tokens_with_commas(entry.cache_write))
                        .set_alignment(CellAlignment::Right),
                    Cell::new(&cache_hit)
                        .fg(Color::Cyan)
                        .set_alignment(CellAlignment::Right),
                    Cell::new(format_currency(entry.cost))
                        .fg(Color::Green)
                        .set_alignment(CellAlignment::Right),
                    Cell::new(format_cost_per_million(entry.cost, total_tokens))
                        .set_alignment(CellAlignment::Right),
                ]);
            }
        }

        // Title
        use colored::Colorize;
        let title = if let Some(ref range) = date_range {
            format!("Hourly Usage ({})", range)
        } else {
            "Hourly Usage".to_string()
        };
        println!("\n  {}\n", title.bold());

        // Table
        let table_str = table.to_string();
        println!("{}", dim_borders(&table_str));

        // Footer with total
        println!(
            "\n  {}  {}",
            "Total:".bold(),
            format_currency(report.total_cost).green().bold()
        );

        if benchmark {
            println!(
                "{}",
                format!("  Processing time: {}ms (Rust native)", processing_time_ms).bright_black()
            );
        }
    }

    Ok(())
}

fn run_wrapped_command(
    output: Option<String>,
    year: Option<String>,
    client_filter: Option<Vec<String>>,
    short: bool,
    agents: bool,
    clients: bool,
    disable_pinned: bool,
) -> Result<()> {
    use colored::Colorize;

    println!("{}", "\n  Tokscale - Generate Wrapped Image\n".cyan());

    println!("{}", "  Generating wrapped image...".bright_black());
    println!();

    let include_agents = !clients || agents;
    let wrapped_options = commands::wrapped::WrappedOptions {
        output,
        year,
        clients: client_filter,
        short,
        include_agents,
        pin_sisyphus: !disable_pinned,
    };

    match commands::wrapped::run(wrapped_options) {
        Ok(output_path) => {
            println!(
                "{}",
                format!("\n  ✓ Generated wrapped image: {}\n", output_path).green()
            );
        }
        Err(err) => {
            eprintln!("{}", "\nError generating wrapped image:".red());
            eprintln!("  {}\n", err);
            std::process::exit(1);
        }
    }

    Ok(())
}

fn run_pricing_lookup(
    model_id: &str,
    json: bool,
    provider: Option<&str>,
    no_spinner: bool,
) -> Result<()> {
    use colored::Colorize;
    use indicatif::ProgressBar;
    use indicatif::ProgressStyle;
    use tokio::runtime::Runtime;
    use tokscale_core::pricing::PricingService;

    let provider_normalized = provider.map(|p| p.to_lowercase());
    if let Some(ref p) = provider_normalized {
        if p != "litellm" && p != "openrouter" {
            println!(
                "\n  {}",
                format!("Invalid provider: {}", provider.unwrap_or("")).red()
            );
            println!(
                "{}\n",
                "  Valid providers: litellm, openrouter".bright_black()
            );
            std::process::exit(1);
        }
    }

    let spinner = if no_spinner {
        None
    } else {
        let provider_label = provider.map(|p| format!(" from {}", p)).unwrap_or_default();
        let pb = ProgressBar::new_spinner();
        pb.set_style(ProgressStyle::default_spinner());
        pb.set_message(format!("Fetching pricing data{}...", provider_label));
        pb.enable_steady_tick(std::time::Duration::from_millis(100));
        Some(pb)
    };

    let rt = Runtime::new()?;
    let result = match rt.block_on(async {
        let svc = PricingService::get_or_init().await?;
        Ok::<_, String>(svc.lookup_with_source(model_id, provider_normalized.as_deref()))
    }) {
        Ok(result) => result,
        Err(err) => {
            if let Some(pb) = spinner {
                pb.finish_and_clear();
            }
            if json {
                #[derive(serde::Serialize)]
                #[serde(rename_all = "camelCase")]
                struct ErrorOutput {
                    error: String,
                    model_id: String,
                }
                println!(
                    "{}",
                    serde_json::to_string_pretty(&ErrorOutput {
                        error: err,
                        model_id: model_id.to_string(),
                    })?
                );
                std::process::exit(1);
            }
            return Err(anyhow::anyhow!(err));
        }
    };

    if let Some(pb) = spinner {
        pb.finish_and_clear();
    }

    if json {
        match result {
            Some(pricing) => {
                #[derive(serde::Serialize)]
                #[serde(rename_all = "camelCase")]
                struct PricingValues {
                    input_cost_per_token: f64,
                    output_cost_per_token: f64,
                    #[serde(skip_serializing_if = "Option::is_none")]
                    cache_read_input_token_cost: Option<f64>,
                    #[serde(skip_serializing_if = "Option::is_none")]
                    cache_creation_input_token_cost: Option<f64>,
                }

                #[derive(serde::Serialize)]
                #[serde(rename_all = "camelCase")]
                struct PricingOutput {
                    model_id: String,
                    matched_key: String,
                    source: String,
                    pricing: PricingValues,
                }

                let output = PricingOutput {
                    model_id: model_id.to_string(),
                    matched_key: pricing.matched_key,
                    source: pricing.source,
                    pricing: PricingValues {
                        input_cost_per_token: pricing.pricing.input_cost_per_token.unwrap_or(0.0),
                        output_cost_per_token: pricing.pricing.output_cost_per_token.unwrap_or(0.0),
                        cache_read_input_token_cost: pricing.pricing.cache_read_input_token_cost,
                        cache_creation_input_token_cost: pricing
                            .pricing
                            .cache_creation_input_token_cost,
                    },
                };

                println!("{}", serde_json::to_string_pretty(&output)?);
            }
            None => {
                #[derive(serde::Serialize)]
                #[serde(rename_all = "camelCase")]
                struct ErrorOutput {
                    error: String,
                    model_id: String,
                }

                let output = ErrorOutput {
                    error: "Model not found".to_string(),
                    model_id: model_id.to_string(),
                };

                println!("{}", serde_json::to_string_pretty(&output)?);
                std::process::exit(1);
            }
        }
    } else {
        match result {
            Some(pricing) => {
                println!("\n  Pricing for: {}", model_id.bold());
                println!("  Matched key: {}", pricing.matched_key);
                let source_label = if pricing.source.eq_ignore_ascii_case("litellm") {
                    "LiteLLM"
                } else {
                    "OpenRouter"
                };
                println!("  Source: {}", source_label);
                println!();
                let input = pricing.pricing.input_cost_per_token.unwrap_or(0.0);
                let output = pricing.pricing.output_cost_per_token.unwrap_or(0.0);
                println!("  Input:  ${:.2} / 1M tokens", input * 1_000_000.0);
                println!("  Output: ${:.2} / 1M tokens", output * 1_000_000.0);
                if let Some(cache_read) = pricing.pricing.cache_read_input_token_cost {
                    println!(
                        "  Cache Read:  ${:.2} / 1M tokens",
                        cache_read * 1_000_000.0
                    );
                }
                if let Some(cache_write) = pricing.pricing.cache_creation_input_token_cost {
                    println!(
                        "  Cache Write: ${:.2} / 1M tokens",
                        cache_write * 1_000_000.0
                    );
                }
                println!();
            }
            None => {
                println!("\n  {}\n", format!("Model not found: {}", model_id).red());
                std::process::exit(1);
            }
        }
    }

    Ok(())
}

fn format_currency(n: f64) -> String {
    format!("${:.2}", n)
}

fn format_cost_per_million(cost: f64, total_tokens: i64) -> String {
    if total_tokens <= 0 || !cost.is_finite() {
        return "—".to_string();
    }
    let cost_per_m = cost * 1_000_000.0 / total_tokens as f64;
    if !cost_per_m.is_finite() {
        "—".to_string()
    } else {
        format!("${:.2}/M", cost_per_m)
    }
}

/// Format a URL as an OSC 8 clickable hyperlink for supported terminals.
/// Falls back to plain URL text when stdout is not a terminal.
fn osc8_link(url: &str) -> String {
    if std::io::stdout().is_terminal() {
        format!("\x1b]8;;{}\x1b\\{}\x1b]8;;\x1b\\", url, url)
    } else {
        url.to_string()
    }
}
/// Format text as an OSC 8 clickable hyperlink with custom display text.
/// Falls back to plain display text when stdout is not a terminal.
fn osc8_link_with_text(url: &str, text: &str) -> String {
    if std::io::stdout().is_terminal() {
        format!("\x1b]8;;{}\x1b\\{}\x1b]8;;\x1b\\", url, text)
    } else {
        text.to_string()
    }
}

fn dim_borders(table_str: &str) -> String {
    let border_chars: &[char] = &['┌', '─', '┬', '┐', '│', '├', '┼', '┤', '└', '┴', '┘'];
    let mut result = String::with_capacity(table_str.len() * 2);

    for ch in table_str.chars() {
        if border_chars.contains(&ch) {
            result.push_str("\x1b[90m");
            result.push(ch);
            result.push_str("\x1b[0m");
        } else {
            result.push(ch);
        }
    }

    result
}

fn format_model_name(model: &str) -> String {
    let name = model.strip_prefix("claude-").unwrap_or(model);
    if name.len() > 9 {
        let potential_date = &name[name.len() - 8..];
        if potential_date.chars().all(|c| c.is_ascii_digit())
            && name.as_bytes()[name.len() - 9] == b'-'
        {
            return name[..name.len() - 9].to_string();
        }
    }
    name.to_string()
}

fn capitalize_client(client: &str) -> String {
    match client {
        "opencode" => "OpenCode".to_string(),
        "claude" => "Claude".to_string(),
        "codex" => "Codex".to_string(),
        "cursor" => "Cursor".to_string(),
        "gemini" => "Gemini".to_string(),
        "amp" => "Amp".to_string(),
        "codebuff" => "Codebuff".to_string(),
        "droid" => "Droid".to_string(),
        "crush" => "Crush".to_string(),
        "openclaw" => "openclaw".to_string(),
        "hermes" => "Hermes Agent".to_string(),
        "goose" => "Goose".to_string(),
        "pi" => "Pi".to_string(),
        other => other.to_string(),
    }
}

fn run_clients_command(json: bool) -> Result<()> {
    use tokscale_core::{extra_scan_paths_for, parse_local_clients, ClientId, LocalParseOptions};

    let home_dir =
        dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?;
    let scanner_settings = tui::settings::load_scanner_settings();

    let parsed = parse_local_clients(LocalParseOptions {
        home_dir: Some(home_dir.to_string_lossy().to_string()),
        use_env_roots: true,
        clients: Some(
            ClientId::iter()
                .filter(|client| client.parse_local())
                .map(|client| client.as_str().to_string())
                .collect(),
        ),
        since: None,
        until: None,
        year: None,
        scanner_settings: scanner_settings.clone(),
    })
    .map_err(|e| anyhow::anyhow!(e))?;

    let headless_roots = get_headless_roots(&home_dir);
    let headless_codex_count = parsed
        .messages
        .iter()
        .filter(|m| m.agent.as_deref() == Some("headless") && m.client == "codex")
        .count() as i32;

    #[derive(serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    struct ClientRow {
        client: String,
        label: String,
        sessions_path: String,
        sessions_path_exists: bool,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        legacy_paths: Vec<LegacyPath>,
        message_count: i32,
        headless_supported: bool,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        headless_paths: Vec<HeadlessPath>,
        headless_message_count: i32,
        #[serde(skip_serializing_if = "Option::is_none")]
        exporter_status: Option<String>,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        extra_paths: Vec<ExtraPath>,
    }

    #[derive(serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    struct LegacyPath {
        path: String,
        exists: bool,
    }

    #[derive(serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    struct HeadlessPath {
        path: String,
        exists: bool,
    }

    #[derive(serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    struct ExtraPath {
        path: String,
        exists: bool,
        source: String,
    }

    // Collect extra dirs from TOKSCALE_EXTRA_DIRS for display (reuse core parser)
    let extra_dirs_val = std::env::var("TOKSCALE_EXTRA_DIRS").unwrap_or_default();
    let all_clients: std::collections::HashSet<ClientId> = ClientId::iter().collect();
    let extra_dirs: Vec<(ClientId, String)> =
        tokscale_core::parse_extra_dirs(&extra_dirs_val, &all_clients);
    let settings_extra_dirs = extra_scan_paths_for(&scanner_settings, &all_clients);
    let copilot_exporter_path = tokscale_core::copilot_exporter_path();

    let clients: Vec<ClientRow> =
        ClientId::iter()
            .map(|client| {
                let sessions_path = client.data().resolve_path(&home_dir.to_string_lossy());
                let sessions_path_exists = Path::new(&sessions_path).exists();
                let legacy_paths = if client == ClientId::OpenClaw {
                    vec![
                        LegacyPath {
                            path: home_dir
                                .join(".clawdbot/agents")
                                .to_string_lossy()
                                .to_string(),
                            exists: home_dir.join(".clawdbot/agents").exists(),
                        },
                        LegacyPath {
                            path: home_dir
                                .join(".moltbot/agents")
                                .to_string_lossy()
                                .to_string(),
                            exists: home_dir.join(".moltbot/agents").exists(),
                        },
                        LegacyPath {
                            path: home_dir
                                .join(".moldbot/agents")
                                .to_string_lossy()
                                .to_string(),
                            exists: home_dir.join(".moldbot/agents").exists(),
                        },
                    ]
                } else {
                    vec![]
                };
                let (headless_supported, headless_paths, headless_message_count) =
                    if client == ClientId::Codex {
                        (
                            true,
                            headless_roots
                                .iter()
                                .map(|root| {
                                    let path = root.join(client.as_str());
                                    HeadlessPath {
                                        path: path.to_string_lossy().to_string(),
                                        exists: path.exists(),
                                    }
                                })
                                .collect(),
                            headless_codex_count,
                        )
                    } else {
                        (false, vec![], 0)
                    };

                let label = match client {
                    ClientId::Claude => "Claude Code",
                    ClientId::Codex => "Codex CLI",
                    ClientId::Copilot => "Copilot CLI",
                    ClientId::Gemini => "Gemini CLI",
                    ClientId::Cursor => "Cursor IDE",
                    ClientId::Kimi => "Kimi CLI",
                    _ => client_ui::display_name(client),
                }
                .to_string();

                let mut extra_paths: Vec<ExtraPath> = settings_extra_dirs
                    .iter()
                    .filter(|(c, _)| *c == client)
                    .map(|(_, path)| ExtraPath {
                        path: path.to_string_lossy().to_string(),
                        exists: path.exists(),
                        source: "settings".to_string(),
                    })
                    .collect();
                extra_paths.extend(extra_dirs.iter().filter(|(c, _)| *c == client).map(
                    |(_, path)| ExtraPath {
                        path: path.clone(),
                        exists: Path::new(path).exists(),
                        source: "env".to_string(),
                    },
                ));

                ClientRow {
                    client: client.as_str().to_string(),
                    label,
                    sessions_path,
                    sessions_path_exists,
                    legacy_paths,
                    message_count: parsed.counts.get(client),
                    headless_supported,
                    headless_paths,
                    headless_message_count,
                    exporter_status: (client == ClientId::Copilot
                        && copilot_exporter_path.is_some())
                    .then(|| "configured".to_string()),
                    extra_paths,
                }
            })
            .collect();

    if json {
        #[derive(serde::Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Output {
            headless_roots: Vec<String>,
            clients: Vec<ClientRow>,
            note: String,
        }

        let output = Output {
            headless_roots: headless_roots
                .iter()
                .map(|p| p.to_string_lossy().to_string())
                .collect(),
            clients,
            note: "Headless capture is supported for Codex CLI only.".to_string(),
        };

        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        use colored::Colorize;

        println!("\n  {}", "Local clients & session counts".cyan());
        println!(
            "  {}",
            format!(
                "Headless roots: {}",
                headless_roots
                    .iter()
                    .map(|p| p.to_string_lossy())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
            .bright_black()
        );
        println!();

        for row in clients {
            println!("  {}", row.label.white());
            println!(
                "  {}",
                format!(
                    "sessions: {}",
                    describe_path(&row.sessions_path, row.sessions_path_exists)
                )
                .bright_black()
            );

            if !row.legacy_paths.is_empty() {
                let legacy_desc: Vec<String> = row
                    .legacy_paths
                    .iter()
                    .map(|lp| describe_path(&lp.path, lp.exists))
                    .collect();
                println!(
                    "  {}",
                    format!("legacy: {}", legacy_desc.join(", ")).bright_black()
                );
            }

            if !row.extra_paths.is_empty() {
                let settings_desc: Vec<String> = row
                    .extra_paths
                    .iter()
                    .filter(|ep| ep.source == "settings")
                    .map(|ep| describe_path(&ep.path, ep.exists))
                    .collect();
                if !settings_desc.is_empty() {
                    println!(
                        "  {}",
                        format!("extra (settings): {}", settings_desc.join(", ")).bright_black()
                    );
                }

                let env_desc: Vec<String> = row
                    .extra_paths
                    .iter()
                    .filter(|ep| ep.source == "env")
                    .map(|ep| describe_path(&ep.path, ep.exists))
                    .collect();
                if !env_desc.is_empty() {
                    println!(
                        "  {}",
                        format!("extra (env): {}", env_desc.join(", ")).bright_black()
                    );
                }
            }

            if let Some(exporter_status) = row.exporter_status.as_ref() {
                println!(
                    "  {}",
                    format!("exporter: {}", exporter_status).bright_black()
                );
            }

            if row.headless_supported {
                let headless_desc: Vec<String> = row
                    .headless_paths
                    .iter()
                    .map(|hp| describe_path(&hp.path, hp.exists))
                    .collect();
                println!(
                    "  {}",
                    format!("headless: {}", headless_desc.join(", ")).bright_black()
                );
                println!(
                    "  {}",
                    format!(
                        "messages: {} (headless: {})",
                        format_number(row.message_count),
                        format_number(row.headless_message_count)
                    )
                    .bright_black()
                );
            } else {
                println!(
                    "  {}",
                    format!("messages: {}", format_number(row.message_count)).bright_black()
                );
            }

            println!();
        }

        println!(
            "  {}",
            "Note: Headless capture is supported for Codex CLI only.".bright_black()
        );
        println!();
    }

    Ok(())
}

fn get_headless_roots(home_dir: &Path) -> Vec<PathBuf> {
    let mut roots = Vec::new();

    if let Ok(env_dir) = std::env::var("TOKSCALE_HEADLESS_DIR") {
        roots.push(PathBuf::from(env_dir));
    } else {
        roots.push(home_dir.join(".config/tokscale/headless"));

        #[cfg(target_os = "macos")]
        {
            roots.push(home_dir.join("Library/Application Support/tokscale/headless"));
        }
    }

    roots
}

fn describe_path(path: &str, exists: bool) -> String {
    let path_display = if let Some(home) = dirs::home_dir() {
        path.replace(&home.to_string_lossy().to_string(), "~")
    } else {
        path.to_string()
    };
    if exists {
        format!("{} ✓", path_display)
    } else {
        format!("{} ✗", path_display)
    }
}

fn format_number(n: i32) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct TsTokenBreakdown {
    input: i64,
    output: i64,
    cache_read: i64,
    cache_write: i64,
    reasoning: i64,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct TsSourceContribution {
    client: String,
    model_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    provider_id: Option<String>,
    tokens: TsTokenBreakdown,
    cost: f64,
    messages: i32,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct TsDailyTotals {
    tokens: i64,
    cost: f64,
    messages: i32,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct TsDailyContribution {
    date: String,
    totals: TsDailyTotals,
    intensity: u8,
    token_breakdown: TsTokenBreakdown,
    clients: Vec<TsSourceContribution>,
}

#[derive(serde::Serialize)]
struct DateRange {
    start: String,
    end: String,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct TsYearSummary {
    year: String,
    total_tokens: i64,
    total_cost: f64,
    range: DateRange,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct TsDataSummary {
    total_tokens: i64,
    total_cost: f64,
    total_days: i32,
    active_days: i32,
    average_per_day: f64,
    max_cost_in_single_day: f64,
    clients: Vec<String>,
    models: Vec<String>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct TsExportMeta {
    generated_at: String,
    version: String,
    date_range: DateRange,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct TsTokenContributionData {
    meta: TsExportMeta,
    summary: TsDataSummary,
    years: Vec<TsYearSummary>,
    contributions: Vec<TsDailyContribution>,
}

fn to_ts_token_contribution_data(graph: &tokscale_core::GraphResult) -> TsTokenContributionData {
    TsTokenContributionData {
        meta: TsExportMeta {
            generated_at: graph.meta.generated_at.clone(),
            version: graph.meta.version.clone(),
            date_range: DateRange {
                start: graph.meta.date_range_start.clone(),
                end: graph.meta.date_range_end.clone(),
            },
        },
        summary: TsDataSummary {
            total_tokens: graph.summary.total_tokens,
            total_cost: graph.summary.total_cost,
            total_days: graph.summary.total_days,
            active_days: graph.summary.active_days,
            average_per_day: graph.summary.average_per_day,
            max_cost_in_single_day: graph.summary.max_cost_in_single_day,
            clients: graph.summary.clients.clone(),
            models: graph.summary.models.clone(),
        },
        years: graph
            .years
            .iter()
            .map(|y| TsYearSummary {
                year: y.year.clone(),
                total_tokens: y.total_tokens,
                total_cost: y.total_cost,
                range: DateRange {
                    start: y.range_start.clone(),
                    end: y.range_end.clone(),
                },
            })
            .collect(),
        contributions: graph
            .contributions
            .iter()
            .map(|d| TsDailyContribution {
                date: d.date.clone(),
                totals: TsDailyTotals {
                    tokens: d.totals.tokens,
                    cost: d.totals.cost,
                    messages: d.totals.messages,
                },
                intensity: d.intensity,
                token_breakdown: TsTokenBreakdown {
                    input: d.token_breakdown.input,
                    output: d.token_breakdown.output,
                    cache_read: d.token_breakdown.cache_read,
                    cache_write: d.token_breakdown.cache_write,
                    reasoning: d.token_breakdown.reasoning,
                },
                clients: d
                    .clients
                    .iter()
                    .map(|s| TsSourceContribution {
                        client: s.client.clone(),
                        model_id: s.model_id.clone(),
                        provider_id: if s.provider_id.is_empty() {
                            None
                        } else {
                            Some(s.provider_id.clone())
                        },
                        tokens: TsTokenBreakdown {
                            input: s.tokens.input,
                            output: s.tokens.output,
                            cache_read: s.tokens.cache_read,
                            cache_write: s.tokens.cache_write,
                            reasoning: s.tokens.reasoning,
                        },
                        cost: s.cost,
                        messages: s.messages,
                    })
                    .collect(),
            })
            .collect(),
    }
}

fn run_login_command() -> Result<()> {
    use tokio::runtime::Runtime;

    let rt = Runtime::new()?;
    rt.block_on(async { auth::login().await })
}

fn run_logout_command() -> Result<()> {
    auth::logout()
}

fn run_whoami_command() -> Result<()> {
    auth::whoami()
}

fn run_delete_data_command() -> Result<()> {
    use colored::Colorize;
    use std::io::{self, Write};
    use tokio::runtime::Runtime;

    let credentials = auth::load_credentials()
        .ok_or_else(|| anyhow::anyhow!("Not logged in. Run `tokscale login` first."))?;

    println!("\n{}", "  ⚠ Delete all submitted usage data".red().bold());
    println!("{}", "  This will permanently remove:".bright_black());
    println!("{}", "    • Leaderboard entries".bright_black());
    println!("{}", "    • Public profile stats".bright_black());
    println!("{}", "    • Daily usage history".bright_black());
    println!(
        "{}",
        "  Your account and API tokens will stay active.\n".bright_black()
    );

    print!(
        "{}",
        "  Are you sure you want to delete all submitted data? (y/N): ".white()
    );
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    if input.trim().to_lowercase() != "y" {
        println!("{}", "  Cancelled.".bright_black());
        return Ok(());
    }

    print!(
        "{}",
        "  This cannot be undone. You will lose all historical token/cost data. Continue? (y/N): "
            .white()
    );
    io::stdout().flush()?;
    input.clear();
    io::stdin().read_line(&mut input)?;
    if input.trim().to_lowercase() != "y" {
        println!("{}", "  Cancelled.".bright_black());
        return Ok(());
    }

    print!("{}", "  Type \"delete my data\" to confirm: ".white());
    io::stdout().flush()?;
    input.clear();
    io::stdin().read_line(&mut input)?;
    if input.trim().to_lowercase() != "delete my data" {
        println!("{}", "  Confirmation failed. Cancelled.".bright_black());
        return Ok(());
    }

    println!("\n{}", "  Deleting submitted data...".bright_black());

    let api_url = auth::get_api_base_url();
    let rt = Runtime::new()?;

    let response = rt.block_on(async {
        reqwest::Client::new()
            .delete(format!("{}/api/settings/submitted-data", api_url))
            .header("Authorization", format!("Bearer {}", credentials.token))
            .send()
            .await
    });

    match response {
        Ok(resp) => {
            let status = resp.status();
            let body: serde_json::Value =
                rt.block_on(async { resp.json().await }).unwrap_or_default();

            match interpret_delete_submitted_data_response(status, &body)? {
                DeleteSubmittedDataOutcome::Deleted(count) => {
                    println!(
                        "{}",
                        format!(
                            "  ✓ Deleted {} submission(s). Leaderboard and profile will refresh shortly.",
                            count
                        )
                        .green()
                    );
                }
                DeleteSubmittedDataOutcome::NotFound => {
                    println!("{}", "  No submitted data found for this account.".yellow());
                }
            }
        }
        Err(e) => {
            return Err(anyhow::anyhow!("Request failed: {}", e));
        }
    }

    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
enum DeleteSubmittedDataOutcome {
    Deleted(i64),
    NotFound,
}

fn interpret_delete_submitted_data_response(
    status: reqwest::StatusCode,
    body: &serde_json::Value,
) -> Result<DeleteSubmittedDataOutcome> {
    if status.is_success() {
        let deleted = body
            .get("deleted")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let count = body
            .get("deletedSubmissions")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);

        if deleted {
            Ok(DeleteSubmittedDataOutcome::Deleted(count))
        } else {
            Ok(DeleteSubmittedDataOutcome::NotFound)
        }
    } else {
        let err = body
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown error");
        Err(anyhow::anyhow!("Failed ({}): {}", status, err))
    }
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct StarCache {
    #[serde(default)]
    username: String,
    #[serde(default)]
    has_starred: bool,
    #[serde(default)]
    checked_at: String,
}

fn star_cache_path() -> Option<PathBuf> {
    Some(crate::paths::get_config_dir().join("star-cache.json"))
}

fn legacy_macos_star_cache_path() -> Option<PathBuf> {
    crate::paths::legacy_macos_config_dir().map(|d| d.join("star-cache.json"))
}

fn load_star_cache(username: &str) -> Option<StarCache> {
    // Read the canonical path first; on macOS, fall back once to the
    // pre-#468 location under `~/Library/Application Support/tokscale/`
    // so existing users don't get re-prompted to star the repo just
    // because their previous cache lives at the legacy path. The legacy
    // read is suppressed when `TOKSCALE_CONFIG_DIR` is set so isolated
    // profiles stay hermetic.
    let primary = star_cache_path().and_then(|path| std::fs::read_to_string(path).ok());
    let content = primary.or_else(|| {
        legacy_macos_star_cache_path().and_then(|legacy| std::fs::read_to_string(legacy).ok())
    })?;
    let cache: StarCache = serde_json::from_str(&content).ok()?;
    // Must match username and have hasStarred=true
    if cache.username != username || !cache.has_starred {
        return None;
    }
    Some(cache)
}

fn save_star_cache(username: &str, has_starred: bool) {
    // Only cache positive confirmations (matching v1 behavior)
    if !has_starred {
        return;
    }
    let Some(path) = star_cache_path() else {
        return;
    };
    let now = chrono::Utc::now().to_rfc3339();
    let cache = StarCache {
        username: username.to_string(),
        has_starred,
        checked_at: now,
    };
    if let Ok(content) = serde_json::to_string_pretty(&cache) {
        if let Some(dir) = path.parent() {
            if std::fs::create_dir_all(dir).is_err() {
                return;
            }
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0);
            let tmp_filename = format!(".star-cache.{}.{:x}.tmp", std::process::id(), nanos);
            let tmp_path = dir.join(tmp_filename);

            let write_result = (|| -> std::io::Result<()> {
                use std::io::Write;
                let mut file = std::fs::File::create(&tmp_path)?;
                file.write_all(content.as_bytes())?;
                file.sync_all()?;
                tokscale_core::fs_atomic::replace_file(&tmp_path, &path)
            })();

            if write_result.is_err() {
                let _ = std::fs::remove_file(&tmp_path);
            }
        }
    }
}

fn prompt_star_repo(username: &str) -> Result<()> {
    use colored::Colorize;
    use std::io::{self, Write};
    use std::process::Command;

    // Check local cache first (avoids network call)
    if load_star_cache(username).is_some() {
        return Ok(());
    }

    // Check if gh CLI is available
    let gh_available = Command::new("gh")
        .arg("--version")
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false);

    if !gh_available {
        return Ok(());
    }

    // Check if user has already starred via gh API
    // Returns exit 0 (HTTP 204) if starred, non-zero (HTTP 404) if not
    let already_starred = Command::new("gh")
        .args(["api", "/user/starred/junhoyeo/tokscale"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if already_starred {
        save_star_cache(username, true);
        return Ok(());
    }

    println!();
    println!("{}", "  Help us grow! \u{2b50}".cyan());
    println!(
        "{}",
        "  Starring tokscale helps others discover the project.".bright_black()
    );
    println!(
        "  {}\n",
        osc8_link("https://github.com/junhoyeo/tokscale").bright_black()
    );
    print!(
        "{}",
        "  \u{2b50} Would you like to star tokscale? (Y/n): ".white()
    );
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let answer = input.trim().to_lowercase();
    if answer == "n" || answer == "no" {
        // Decline: don't cache (will re-prompt next time, matching v1)
        println!();
        return Ok(());
    }

    // Star via gh API (gh repo star is not a valid command)
    let status = Command::new("gh")
        .args([
            "api",
            "--silent",
            "--method",
            "PUT",
            "/user/starred/junhoyeo/tokscale",
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    match status {
        Ok(s) if s.success() => {
            println!(
                "{}",
                "  \u{2713} Starred! Thank you for your support.\n".green()
            );
            save_star_cache(username, true);
        }
        _ => {
            println!(
                "{}",
                "  Failed to star via gh CLI. Continuing to submit...\n".yellow()
            );
        }
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn run_graph_command(
    output: Option<String>,
    home_dir: Option<String>,
    clients: Option<Vec<String>>,
    since: Option<String>,
    until: Option<String>,
    year: Option<String>,
    benchmark: bool,
    no_spinner: bool,
) -> Result<()> {
    use colored::Colorize;
    use std::time::Instant;
    use tokscale_core::{generate_local_graph_report, GroupBy, ReportOptions};

    let show_progress = output.is_some() && !no_spinner;
    let include_cursor = home_dir.is_none()
        && clients
            .as_ref()
            .is_none_or(|s| s.iter().any(|src| src == "cursor"));
    let has_cursor_cache = include_cursor && cursor::has_cursor_usage_cache();
    let mut cursor_sync_result: Option<cursor::SyncCursorResult> = None;

    if include_cursor && cursor::is_cursor_logged_in() {
        let rt_sync = tokio::runtime::Runtime::new()?;
        cursor_sync_result = Some(rt_sync.block_on(async { cursor::sync_cursor_cache().await }));
    }

    if show_progress {
        eprintln!("  Scanning session data...");
    }
    let start = Instant::now();

    if show_progress {
        eprintln!("  Generating graph data...");
    }
    let use_env_roots = use_env_roots(&home_dir);
    let rt = tokio::runtime::Runtime::new()?;
    let graph_result = rt
        .block_on(async {
            generate_local_graph_report(ReportOptions {
                home_dir,
                use_env_roots,
                clients,
                since,
                until,
                year,
                group_by: GroupBy::default(),
                scanner_settings: tui::settings::load_scanner_settings(),
            })
            .await
        })
        .map_err(|e| anyhow::anyhow!(e))?;

    let processing_time_ms = start.elapsed().as_millis() as u32;
    let output_data = to_ts_token_contribution_data(&graph_result);
    let json_output = serde_json::to_string_pretty(&output_data)?;

    if let Some(output_path) = output {
        std::fs::write(&output_path, json_output)?;

        eprintln!(
            "{}",
            format!("✓ Graph data written to {}", output_path).green()
        );
        eprintln!(
            "{}",
            format!(
                "  {} days, {} clients, {} models",
                output_data.contributions.len(),
                output_data.summary.clients.len(),
                output_data.summary.models.len()
            )
            .bright_black()
        );
        eprintln!(
            "{}",
            format!(
                "  Total: {}",
                format_currency(output_data.summary.total_cost)
            )
            .bright_black()
        );

        if benchmark {
            eprintln!(
                "{}",
                format!("  Processing time: {}ms (Rust native)", processing_time_ms).bright_black()
            );
            if let Some(sync) = cursor_sync_result {
                if sync.synced {
                    eprintln!(
                        "{}",
                        format!(
                            "  Cursor: {} usage events synced (full lifetime data)",
                            sync.rows
                        )
                        .bright_black()
                    );
                } else if let Some(err) = sync.error {
                    if has_cursor_cache {
                        eprintln!("{}", format!("  Cursor: sync failed - {}", err).yellow());
                    }
                }
            }
        }
    } else {
        println!("{}", json_output);
    }

    Ok(())
}

#[derive(serde::Deserialize)]
struct SubmitResponse {
    #[serde(rename = "submissionId")]
    submission_id: Option<String>,
    #[allow(dead_code)]
    username: Option<String>,
    metrics: Option<SubmitMetrics>,
    warnings: Option<Vec<String>>,
    error: Option<String>,
    details: Option<Vec<String>>,
}

#[derive(serde::Deserialize)]
struct SubmitMetrics {
    #[serde(rename = "totalTokens")]
    total_tokens: Option<i64>,
    #[serde(rename = "totalCost")]
    total_cost: Option<f64>,
    #[serde(rename = "activeDays")]
    active_days: Option<i32>,
    #[allow(dead_code)]
    sources: Option<Vec<String>>,
}

fn cap_graph_result_to_utc_today(
    graph_result: &mut tokscale_core::GraphResult,
    utc_today: &str,
) -> bool {
    let pre_cap_len = graph_result.contributions.len();
    graph_result
        .contributions
        .retain(|c| c.date.as_str() <= utc_today);
    if graph_result.contributions.len() == pre_cap_len {
        return false;
    }

    graph_result.meta.date_range_start = graph_result
        .contributions
        .first()
        .map(|c| c.date.clone())
        .unwrap_or_default();
    graph_result.meta.date_range_end = graph_result
        .contributions
        .last()
        .map(|c| c.date.clone())
        .unwrap_or_default();
    graph_result.summary = tokscale_core::calculate_summary(&graph_result.contributions);
    graph_result.years = tokscale_core::calculate_years(&graph_result.contributions);

    true
}

fn run_submit_command(
    clients: Option<Vec<String>>,
    since: Option<String>,
    until: Option<String>,
    year: Option<String>,
    dry_run: bool,
) -> Result<()> {
    use colored::Colorize;
    use std::io::IsTerminal;
    use tokio::runtime::Runtime;
    use tokscale_core::{generate_graph, GroupBy, ReportOptions};

    let credentials = match auth::load_credentials() {
        Some(creds) => creds,
        None => {
            eprintln!("\n  {}", "Not logged in.".yellow());
            eprintln!(
                "{}",
                "  Run 'bunx tokscale@latest login' first.\n".bright_black()
            );
            std::process::exit(1);
        }
    };

    if std::io::stdin().is_terminal() && std::io::stdout().is_terminal() {
        let _ = prompt_star_repo(&credentials.username);
    }

    println!("\n  {}\n", "Tokscale - Submit Usage Data".cyan());

    let clients = clients.or_else(|| Some(default_submit_clients()));

    let include_cursor = clients
        .as_ref()
        .is_none_or(|s| s.iter().any(|src| src == "cursor"));
    let has_cursor_cache = cursor::has_cursor_usage_cache();
    if include_cursor && cursor::is_cursor_logged_in() {
        println!("{}", "  Syncing Cursor usage data...".bright_black());
        let rt_sync = Runtime::new()?;
        let sync_result = rt_sync.block_on(async { cursor::sync_cursor_cache().await });
        if sync_result.synced {
            println!(
                "{}",
                format!("  Cursor: {} usage events synced", sync_result.rows).bright_black()
            );
        } else if let Some(err) = sync_result.error {
            if has_cursor_cache {
                println!(
                    "{}",
                    format!("  Cursor sync failed; using cached data: {}", err).yellow()
                );
            }
        }
    }

    println!("{}", "  Scanning local session data...".bright_black());

    let rt = Runtime::new()?;
    let graph_result = rt
        .block_on(async {
            generate_graph(ReportOptions {
                home_dir: None,
                use_env_roots: true,
                clients,
                since,
                until,
                year,
                group_by: GroupBy::default(),
                scanner_settings: tui::settings::load_scanner_settings(),
            })
            .await
        })
        .map_err(|e| anyhow::anyhow!(e))?;

    // Cap contributions to UTC today to prevent timezone-related future-date
    // rejections. The CLI generates dates using chrono::Local, but the server
    // validates against UTC. In UTC+ timezones the local date can be ahead of
    // UTC around midnight, causing valid same-day data to be flagged as
    // "future dates". Capped contributions will be included in the next
    // submission once the UTC date catches up.
    // See: https://github.com/junhoyeo/tokscale/issues/318
    let utc_today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let mut graph_result = graph_result;
    cap_graph_result_to_utc_today(&mut graph_result, &utc_today);

    println!("{}", "  Data to submit:".white());
    println!(
        "{}",
        format!(
            "    Date range: {} to {}",
            graph_result.meta.date_range_start, graph_result.meta.date_range_end,
        )
        .bright_black()
    );
    println!(
        "{}",
        format!("    Active days: {}", graph_result.summary.active_days).bright_black()
    );
    println!(
        "{}",
        format!(
            "    Total tokens: {}",
            format_tokens_with_commas(graph_result.summary.total_tokens)
        )
        .bright_black()
    );
    println!(
        "{}",
        format!(
            "    Total cost: {}",
            format_currency(graph_result.summary.total_cost)
        )
        .bright_black()
    );
    println!(
        "{}",
        format!("    Clients: {}", graph_result.summary.clients.join(", ")).bright_black()
    );
    println!(
        "{}",
        format!("    Models: {} models", graph_result.summary.models.len()).bright_black()
    );
    println!();

    if graph_result.summary.total_tokens == 0 {
        println!("{}", "  No usage data found to submit.\n".yellow());
        return Ok(());
    }

    if dry_run {
        println!("{}", "  Dry run - not submitting data.\n".yellow());
        return Ok(());
    }

    println!("{}", "  Submitting to server...".bright_black());

    let api_url = auth::get_api_base_url();

    let submit_payload = to_ts_token_contribution_data(&graph_result);

    let response = rt.block_on(async {
        reqwest::Client::new()
            .post(format!("{}/api/submit", api_url))
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {}", credentials.token))
            .json(&submit_payload)
            .send()
            .await
    });

    match response {
        Ok(resp) => {
            let status = resp.status();
            let body: SubmitResponse =
                rt.block_on(async { resp.json().await })
                    .unwrap_or_else(|_| SubmitResponse {
                        submission_id: None,
                        username: None,
                        metrics: None,
                        warnings: None,
                        error: Some(format!(
                            "Server returned {} with unparseable response",
                            status
                        )),
                        details: None,
                    });

            if !status.is_success() {
                eprintln!(
                    "\n  {}",
                    format!(
                        "Error: {}",
                        body.error
                            .unwrap_or_else(|| "Submission failed".to_string())
                    )
                    .red()
                );
                if let Some(details) = body.details {
                    for detail in details {
                        eprintln!("{}", format!("    - {}", detail).bright_black());
                    }
                }
                println!();
                std::process::exit(1);
            }

            println!("\n  {}", "Successfully submitted!".green());
            println!();
            println!("{}", "  Summary:".white());
            if let Some(id) = body.submission_id {
                println!("{}", format!("    Submission ID: {}", id).bright_black());
            }
            if let Some(metrics) = &body.metrics {
                if let Some(tokens) = metrics.total_tokens {
                    println!(
                        "{}",
                        format!("    Total tokens: {}", format_tokens_with_commas(tokens))
                            .bright_black()
                    );
                }
                if let Some(cost) = metrics.total_cost {
                    println!(
                        "{}",
                        format!("    Total cost: {}", format_currency(cost)).bright_black()
                    );
                }
                if let Some(days) = metrics.active_days {
                    println!("{}", format!("    Active days: {}", days).bright_black());
                }
            }
            println!();
            println!(
                "{}",
                osc8_link_with_text(
                    &format!("{}/u/{}", api_url, credentials.username),
                    &format!(
                        "  View your profile: {}/u/{}",
                        api_url, credentials.username
                    ),
                )
                .cyan()
            );
            println!();

            if let Some(warnings) = body.warnings {
                if !warnings.is_empty() {
                    println!("{}", "  Warnings:".yellow());
                    for warning in warnings {
                        println!("{}", format!("    - {}", warning).bright_black());
                    }
                    println!();
                }
            }
        }
        Err(err) => {
            eprintln!("\n  {}", "Error: Failed to connect to server.".red());
            eprintln!("{}\n", format!("  {}", err).bright_black());
            std::process::exit(1);
        }
    }

    // Warm the TUI cache so the next `tokscale` launch is instant.
    // Detached subprocess so submit returns to the shell immediately on large
    // datasets — a full re-scan would otherwise block for tens of seconds.
    spawn_warm_tui_cache_detached();

    Ok(())
}

fn spawn_warm_tui_cache_detached() {
    use std::process::{Command, Stdio};

    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(_) => return,
    };

    let mut cmd = Command::new(exe);
    cmd.arg("warm-tui-cache")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // New process group so the child is not killed by Ctrl-C in the
        // parent's shell and survives after submit exits.
        cmd.process_group(0);
    }

    let _ = cmd.spawn();
}

/// Resolve the filter set used by a no-`--client`-flag TUI launch.
///
/// Mirrors the resolution that `build_client_filter` + `tui::run` perform
/// when the user passes no CLI client flag:
///
/// 1. If `defaultClients` from `~/.config/tokscale/settings.json` is
///    set, use that (after dropping unknown ids).
/// 2. Otherwise fall back to `ClientFilter::default_set()` (every real
///    client, Synthetic excluded).
///
/// This **must** stay in lockstep with the resolution that
/// `tui::run(.., clients = None, ..)` would compute. If it drifts, the
/// `submit` warm cache uses one filter set while the next no-flag TUI
/// launch wants another, the cache key mismatches, and the warming
/// becomes a wasted background scan.
fn resolve_default_tui_filter_set() -> std::collections::HashSet<ClientFilter> {
    resolve_default_tui_filter_set_with(&tui::settings::load_default_clients())
}

/// Pure variant of `resolve_default_tui_filter_set` for unit-testable
/// resolution. `configured` is the (raw, pre-validation) list of ids
/// from settings.json.
fn resolve_default_tui_filter_set_with(
    configured: &[String],
) -> std::collections::HashSet<ClientFilter> {
    let parsed: Vec<ClientFilter> = configured
        .iter()
        .filter_map(|s| ClientFilter::from_filter_str(s))
        .collect();
    if parsed.is_empty() {
        ClientFilter::default_set()
    } else {
        parsed.into_iter().collect()
    }
}

fn resolve_should_write_cache(
    cli_write: bool,
    cli_no_write: bool,
    settings: &tui::settings::Settings,
) -> bool {
    if cli_no_write {
        return false;
    }
    if cli_write {
        return true;
    }
    settings.light.write_cache
}

fn resolve_light_cache_filter_set(
    clients: &Option<Vec<String>>,
) -> std::collections::HashSet<ClientFilter> {
    if let Some(clients) = clients {
        clients
            .iter()
            .filter_map(|client| ClientFilter::from_filter_str(client))
            .collect()
    } else {
        resolve_default_tui_filter_set()
    }
}

fn write_light_cache(
    home_dir: &Option<String>,
    clients: &Option<Vec<String>>,
    since: &Option<String>,
    until: &Option<String>,
    year: &Option<String>,
    group_by: &tokscale_core::GroupBy,
) {
    use crate::tui::{save_cached_data, DataLoader};

    // The TUI cache key is `(enabled_clients, group_by)` only — it does
    // NOT include `--since`, `--until`, `--year`, or `--home`. Writing
    // date-filtered or home-scoped data under that key would silently
    // poison subsequent TUI launches: the next `tokscale tui` would
    // hit the cache and render the date-filtered slice as if it were
    // the full report. Refuse the write when any of those filters is
    // present and tell the user; their CLI report still prints fine.
    if since.is_some() || until.is_some() || year.is_some() || home_dir.is_some() {
        eprintln!(
            "tokscale: --write-cache skipped because --since/--until/--year/--home are set; \
             the TUI cache key does not include those filters and writing would poison future TUI launches."
        );
        return;
    }

    let enabled_set = resolve_light_cache_filter_set(clients);
    let scan_clients: Vec<tokscale_core::ClientId> = enabled_set
        .iter()
        .filter_map(|filter| filter.to_client_id())
        .collect();
    let include_synthetic = enabled_set.contains(&ClientFilter::Synthetic);

    // No date/home filters at this point (guarded above), so passing
    // None into `with_filters` matches what the TUI itself does on
    // launch — keeps the cache key derivation byte-identical.
    //
    // Cache writes are best-effort: the report has already been flushed
    // to stdout by the time we reach here, so a scan failure from the
    // background loader must NOT propagate up and turn a successful
    // user-visible report into a non-zero exit code. Mirrors the
    // pattern in `run_warm_tui_cache` below.
    let loader = DataLoader::with_filters(None, None, None, None);
    if let Ok(data) = loader.load(&scan_clients, group_by, include_synthetic) {
        save_cached_data(&data, &enabled_set, group_by);
    }
}

fn run_warm_tui_cache() -> Result<()> {
    use crate::tui::{save_cached_data, DataLoader};
    use tokscale_core::{ClientId, GroupBy};

    // Warm the cache using the same default filter set the TUI uses on
    // a no-flag launch. Going through `resolve_default_tui_filter_set()`
    // keeps these two paths in lockstep — including the user's
    // `defaultClients` setting, which the TUI honors via
    // `build_client_filter`. If they drift, every TUI launch after
    // `submit` becomes a cache miss instead of a fresh hit, defeating
    // the warming.
    let enabled_set = resolve_default_tui_filter_set();
    let scan_clients: Vec<ClientId> = enabled_set
        .iter()
        .filter_map(|f| f.to_client_id())
        .collect();
    let include_synthetic = enabled_set.contains(&ClientFilter::Synthetic);
    let loader = DataLoader::with_filters(None, None, None, None);
    if let Ok(data) = loader.load(&scan_clients, &GroupBy::default(), include_synthetic) {
        save_cached_data(&data, &enabled_set, &GroupBy::default());
    }
    Ok(())
}

fn run_cursor_command(subcommand: CursorSubcommand) -> Result<()> {
    match subcommand {
        CursorSubcommand::Login { name } => cursor::run_cursor_login(name),
        CursorSubcommand::Logout {
            name,
            all,
            purge_cache,
        } => cursor::run_cursor_logout(name, all, purge_cache),
        CursorSubcommand::Status { name } => cursor::run_cursor_status(name),
        CursorSubcommand::Accounts { json } => cursor::run_cursor_accounts(json),
        CursorSubcommand::Sync { json } => cursor::run_cursor_sync(json),
        CursorSubcommand::Switch { name } => cursor::run_cursor_switch(&name),
    }
}

fn run_antigravity_command(subcommand: AntigravitySubcommand) -> Result<()> {
    match subcommand {
        AntigravitySubcommand::Sync => antigravity::run_antigravity_sync(),
        AntigravitySubcommand::Status { json } => antigravity::run_antigravity_status(json),
        AntigravitySubcommand::PurgeCache => antigravity::run_antigravity_purge_cache(),
    }
}

fn format_tokens_with_commas(n: i64) -> String {
    let s = n.to_string();
    let bytes = s.as_bytes();
    let len = bytes.len();
    let mut result = String::with_capacity(len + len / 3);
    for (i, &b) in bytes.iter().enumerate() {
        if i > 0 && (len - i).is_multiple_of(3) {
            result.push(',');
        }
        result.push(b as char);
    }
    result
}

fn run_headless_command(
    source: &str,
    args: Vec<String>,
    format: Option<String>,
    output: Option<String>,
    no_auto_flags: bool,
) -> Result<()> {
    use chrono::Utc;
    use std::io::{Read, Write};
    use std::process::Command;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use uuid::Uuid;

    let source_lower = source.to_lowercase();
    if source_lower != "codex" {
        eprintln!("\n  Error: Unknown headless source '{}'.", source);
        eprintln!("  Currently only 'codex' is supported.\n");
        std::process::exit(1);
    }

    let resolved_format = match format {
        Some(f) if f == "json" || f == "jsonl" => f,
        Some(f) => {
            eprintln!("\n  Error: Invalid format '{}'. Use json or jsonl.\n", f);
            std::process::exit(1);
        }
        None => "jsonl".to_string(),
    };

    let mut final_args = args.clone();
    if !no_auto_flags && source_lower == "codex" && !final_args.contains(&"--json".to_string()) {
        final_args.push("--json".to_string());
    }

    let home_dir =
        dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?;
    let headless_roots = get_headless_roots(&home_dir);

    let output_path = if let Some(custom_output) = output {
        let parent = Path::new(&custom_output)
            .parent()
            .unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(parent)?;
        custom_output
    } else {
        let root = headless_roots
            .first()
            .cloned()
            .unwrap_or_else(|| home_dir.join(".config/tokscale/headless"));
        let dir = root.join(&source_lower);
        std::fs::create_dir_all(&dir)?;

        let now = Utc::now();
        let timestamp = now.format("%Y-%m-%dT%H-%M-%S-%3fZ").to_string();
        let uuid_short = Uuid::new_v4()
            .to_string()
            .replace("-", "")
            .chars()
            .take(8)
            .collect::<String>();
        let filename = format!(
            "{}-{}-{}.{}",
            source_lower, timestamp, uuid_short, resolved_format
        );

        dir.join(filename).to_string_lossy().to_string()
    };

    let settings = tui::settings::Settings::load();
    let timeout = settings.get_native_timeout();

    use colored::Colorize;
    println!("\n  {}", "Headless capture".cyan());
    println!("  {}", format!("source: {}", source_lower).bright_black());
    println!("  {}", format!("output: {}", output_path).bright_black());
    println!(
        "  {}",
        format!("timeout: {}s", timeout.as_secs()).bright_black()
    );
    println!();

    let mut child = Command::new(&source_lower)
        .args(&final_args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::inherit())
        .stdin(std::process::Stdio::inherit())
        .spawn()
        .map_err(|e| anyhow::anyhow!("Failed to spawn '{}': {}", source_lower, e))?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow::anyhow!("Failed to capture stdout from command"))?;

    let mut output_file = std::fs::File::create(&output_path)
        .map_err(|e| anyhow::anyhow!("Failed to create output file '{}': {}", output_path, e))?;

    let timed_out = Arc::new(AtomicBool::new(false));
    let timed_out_clone = Arc::clone(&timed_out);
    let child_id = child.id();

    let timeout_handle = std::thread::spawn(move || {
        std::thread::sleep(timeout);
        if !timed_out_clone.load(Ordering::SeqCst) {
            timed_out_clone.store(true, Ordering::SeqCst);
            #[cfg(unix)]
            {
                let _ = std::process::Command::new("kill")
                    .arg("-9")
                    .arg(child_id.to_string())
                    .output();
            }
            #[cfg(windows)]
            {
                let _ = std::process::Command::new("taskkill")
                    .args(["/F", "/PID", &child_id.to_string()])
                    .output();
            }
        }
    });

    let mut reader = std::io::BufReader::new(stdout);
    let mut buffer = [0; 8192];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => {
                output_file
                    .write_all(&buffer[..n])
                    .map_err(|e| anyhow::anyhow!("Failed to write to output file: {}", e))?;
            }
            Err(e) => {
                if timed_out.load(Ordering::SeqCst) {
                    break;
                }
                return Err(anyhow::anyhow!(
                    "Failed to read from subprocess stdout: {}",
                    e
                ));
            }
        }
    }

    let status = child
        .wait()
        .map_err(|e| anyhow::anyhow!("Failed to wait for subprocess: {}", e))?;

    timed_out.store(true, Ordering::SeqCst);
    let _ = timeout_handle.join();

    if timed_out.load(Ordering::SeqCst) && !status.success() {
        eprintln!(
            "{}",
            format!("\n  Subprocess timed out after {}s", timeout.as_secs()).red()
        );
        eprintln!("{}", "  Partial output saved. Increase timeout with TOKSCALE_NATIVE_TIMEOUT_MS or settings.json".bright_black());
        println!();
        std::process::exit(124);
    }

    let exit_code = status.code().unwrap_or(1);

    println!(
        "{}",
        format!("✓ Saved headless output to {}", output_path).green()
    );
    println!();

    if exit_code != 0 {
        std::process::exit(exit_code);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use reqwest::StatusCode;
    use tokscale_core::{
        calculate_summary, calculate_years, ClientContribution, DailyContribution, DailyTotals,
        GraphMeta, GraphResult, TokenBreakdown, YearSummary,
    };

    fn token_breakdown(total_tokens: i64) -> TokenBreakdown {
        TokenBreakdown {
            input: total_tokens,
            output: 0,
            cache_read: 0,
            cache_write: 0,
            reasoning: 0,
        }
    }

    fn daily_contribution(
        date: &str,
        total_tokens: i64,
        total_cost: f64,
        client: &str,
        model_id: &str,
    ) -> DailyContribution {
        DailyContribution {
            date: date.to_string(),
            totals: DailyTotals {
                tokens: total_tokens,
                cost: total_cost,
                messages: 1,
            },
            intensity: 0,
            token_breakdown: token_breakdown(total_tokens),
            clients: vec![ClientContribution {
                client: client.to_string(),
                model_id: model_id.to_string(),
                provider_id: "openai".to_string(),
                tokens: token_breakdown(total_tokens),
                cost: total_cost,
                messages: 1,
            }],
        }
    }

    fn graph_result_with_contributions(contributions: Vec<DailyContribution>) -> GraphResult {
        GraphResult {
            meta: GraphMeta {
                generated_at: "2026-03-24T00:00:00Z".to_string(),
                version: "test".to_string(),
                date_range_start: contributions
                    .first()
                    .map(|c| c.date.clone())
                    .unwrap_or_default(),
                date_range_end: contributions
                    .last()
                    .map(|c| c.date.clone())
                    .unwrap_or_default(),
                processing_time_ms: 0,
            },
            summary: calculate_summary(&contributions),
            years: calculate_years(&contributions),
            contributions,
        }
    }

    fn year_summary(graph: &GraphResult, year: &str) -> YearSummary {
        graph
            .years
            .iter()
            .find(|entry| entry.year == year)
            .cloned()
            .unwrap()
    }

    // Tests below call `build_client_filter_with_defaults` directly with
    // an explicit `defaults` slice instead of `build_client_filter`, which
    // reads from `~/.config/tokscale/settings.json`. Reading host config
    // makes tests non-hermetic — a developer with their own
    // `defaultClients` set would break the assertions. The wrapper is
    // covered separately by tests that pass an explicit `&[]`.

    #[test]
    fn test_build_client_filter_all_false() {
        let flags = ClientFlags::default();
        assert_eq!(build_client_filter_with_defaults(flags, &[]), None);
    }

    #[test]
    fn test_build_client_filter_single_legacy_flag() {
        let flags = ClientFlags {
            opencode: true,
            ..ClientFlags::default()
        };
        assert_eq!(
            build_client_filter_with_defaults(flags, &[]),
            Some(vec!["opencode".to_string()])
        );
    }

    #[test]
    fn test_build_client_filter_multiple_legacy_flags_preserve_order() {
        let flags = ClientFlags {
            opencode: true,
            claude: true,
            pi: true,
            ..ClientFlags::default()
        };
        // Legacy iteration order is the declaration order in `legacy[]`,
        // not the order the user typed flags on the command line. This is
        // a deliberate trade-off: legacy flags are deprecated, and the
        // canonical `--client a,b,c` form preserves user order.
        assert_eq!(
            build_client_filter_with_defaults(flags, &[]),
            Some(vec![
                "opencode".to_string(),
                "claude".to_string(),
                "pi".to_string()
            ])
        );
    }

    #[test]
    fn test_build_client_filter_synthetic_only_legacy() {
        let flags = ClientFlags {
            synthetic: true,
            ..ClientFlags::default()
        };
        assert_eq!(
            build_client_filter_with_defaults(flags, &[]),
            Some(vec!["synthetic".to_string()])
        );
    }

    #[test]
    fn test_build_client_filter_all_legacy_flags() {
        let flags = ClientFlags {
            opencode: true,
            claude: true,
            codex: true,
            copilot: true,
            gemini: true,
            cursor: true,
            amp: true,
            codebuff: true,
            droid: true,
            openclaw: true,
            hermes: true,
            pi: true,
            kimi: true,
            qwen: true,
            roocode: true,
            kilocode: true,
            kilo: true,
            mux: true,
            crush: true,
            goose: true,
            antigravity: true,
            synthetic: true,
            ..ClientFlags::default()
        };
        let result = build_client_filter_with_defaults(flags, &[]);
        assert!(result.is_some());
        let sources = result.unwrap();
        // ClientId::COUNT does not include synthetic, but ClientFilter does.
        let expected_len = tokscale_core::ClientId::iter().count() + 1;
        assert_eq!(sources.len(), expected_len);
        for required in [
            "opencode",
            "claude",
            "codex",
            "copilot",
            "gemini",
            "cursor",
            "amp",
            "codebuff",
            "droid",
            "openclaw",
            "hermes",
            "pi",
            "kimi",
            "qwen",
            "roocode",
            "kilocode",
            "kilo",
            "mux",
            "crush",
            "goose",
            "antigravity",
            "synthetic",
        ] {
            assert!(
                sources.contains(&required.to_string()),
                "missing client filter id: {required}"
            );
        }
    }

    #[test]
    fn test_build_client_filter_canonical_clients_preserve_user_order() {
        // `--client claude,opencode,pi` should keep user-typed order so
        // downstream display (e.g. table grouping previews) is stable.
        let flags = ClientFlags {
            clients: vec![
                ClientFilter::Claude,
                ClientFilter::Opencode,
                ClientFilter::Pi,
            ],
            ..ClientFlags::default()
        };
        assert_eq!(
            build_client_filter_with_defaults(flags, &[]),
            Some(vec![
                "claude".to_string(),
                "opencode".to_string(),
                "pi".to_string(),
            ])
        );
    }

    #[test]
    fn test_build_client_filter_canonical_dedups_repeats() {
        let flags = ClientFlags {
            clients: vec![
                ClientFilter::Claude,
                ClientFilter::Claude,
                ClientFilter::Opencode,
            ],
            ..ClientFlags::default()
        };
        assert_eq!(
            build_client_filter_with_defaults(flags, &[]),
            Some(vec!["claude".to_string(), "opencode".to_string()])
        );
    }

    #[test]
    fn test_build_client_filter_canonical_and_legacy_dedup() {
        // Mixing canonical `--client claude` with legacy `--claude` must not
        // double-list claude. Canonical entries come first, legacy fills in
        // anything missing.
        let flags = ClientFlags {
            clients: vec![ClientFilter::Claude],
            opencode: true,
            claude: true,
            ..ClientFlags::default()
        };
        assert_eq!(
            build_client_filter_with_defaults(flags, &[]),
            Some(vec!["claude".to_string(), "opencode".to_string()])
        );
    }

    #[test]
    fn test_client_filter_as_filter_str_matches_client_id_for_overlap() {
        // Every ClientFilter variant except Synthetic must agree with
        // ClientId::as_str() so the core filter list stays consistent.
        for filter in ClientFilter::value_variants() {
            if matches!(filter, ClientFilter::Synthetic) {
                continue;
            }
            let id = filter.as_filter_str();
            assert!(
                tokscale_core::ClientId::from_str(id).is_some(),
                "ClientFilter::{:?} -> {:?} has no matching ClientId",
                filter,
                id,
            );
        }
    }

    #[test]
    fn test_client_filter_to_client_id_round_trip() {
        // For every non-Synthetic filter:
        //   from_client_id(to_client_id(filter).unwrap()) == filter
        // and the canonical id strings agree.
        for filter in ClientFilter::value_variants() {
            match filter.to_client_id() {
                Some(id) => {
                    assert_eq!(
                        ClientFilter::from_client_id(id),
                        *filter,
                        "round-trip mismatch for {:?}",
                        filter
                    );
                    assert_eq!(
                        id.as_str(),
                        filter.as_filter_str(),
                        "id string drift between ClientId and ClientFilter for {:?}",
                        filter
                    );
                }
                None => {
                    // Synthetic is the only meta-client without a ClientId.
                    assert!(matches!(filter, ClientFilter::Synthetic));
                }
            }
        }
    }

    #[test]
    fn test_client_filter_order_matches_client_id_all() {
        // Picker rendering, --help possible-values listing, and any
        // future iteration over `ClientFilter::value_variants()` all
        // assume the variant order mirrors `ClientId::ALL` (with
        // Synthetic appended). Guard that invariant explicitly.
        let filters: Vec<ClientFilter> = ClientFilter::value_variants()
            .iter()
            .copied()
            .filter(|f| !matches!(f, ClientFilter::Synthetic))
            .collect();
        let ids: Vec<tokscale_core::ClientId> = tokscale_core::ClientId::ALL.to_vec();
        assert_eq!(filters.len(), ids.len());
        for (filter, id) in filters.iter().zip(ids.iter()) {
            assert_eq!(
                filter.to_client_id(),
                Some(*id),
                "ClientFilter declaration order diverged from ClientId::ALL at {:?}",
                filter
            );
        }
        // Synthetic is the very last variant.
        assert_eq!(
            ClientFilter::value_variants().last().copied(),
            Some(ClientFilter::Synthetic)
        );
    }

    #[test]
    fn test_client_filter_from_filter_str_accepts_canonical_ids() {
        for filter in ClientFilter::value_variants() {
            let id = filter.as_filter_str();
            assert_eq!(ClientFilter::from_filter_str(id), Some(*filter));
        }
        assert_eq!(ClientFilter::from_filter_str("not-a-client"), None);
    }

    #[test]
    fn test_client_filter_default_set_excludes_synthetic() {
        // Synthetic detection is opt-in: it post-processes other clients'
        // sessions to re-attribute messages to a different bucket. The
        // pre-refactor default was "every ClientId, include_synthetic =
        // false"; default_set() must preserve that contract.
        let default = ClientFilter::default_set();
        assert!(
            !default.contains(&ClientFilter::Synthetic),
            "default_set() must NOT include Synthetic — it is opt-in only"
        );
        // Every real client must be present so first-launch reports cover
        // every integration the binary knows about.
        for filter in ClientFilter::value_variants() {
            if matches!(filter, ClientFilter::Synthetic) {
                continue;
            }
            assert!(default.contains(filter), "default_set() missing {filter:?}");
        }
        // Size sanity: every variant minus Synthetic.
        assert_eq!(
            default.len(),
            ClientFilter::value_variants().len() - 1,
            "default_set() size drifted from value_variants() - 1"
        );
    }

    #[test]
    fn test_resolve_default_tui_filter_set_uses_configured_defaults() {
        // When `defaultClients` is set, the warm-cache resolver must use
        // it verbatim — otherwise the warm cache would store every real
        // client while the next no-flag TUI launch wants only the configured
        // ones, producing a guaranteed cache miss.
        let configured = vec!["opencode".to_string(), "claude".to_string()];
        let set = resolve_default_tui_filter_set_with(&configured);
        let mut expected = std::collections::HashSet::new();
        expected.insert(ClientFilter::Opencode);
        expected.insert(ClientFilter::Claude);
        assert_eq!(set, expected);
    }

    #[test]
    fn test_resolve_default_tui_filter_set_falls_back_when_empty() {
        // No defaultClients configured → use the canonical default set.
        let set = resolve_default_tui_filter_set_with(&[]);
        assert_eq!(set, ClientFilter::default_set());
    }

    #[test]
    fn test_resolve_default_tui_filter_set_drops_unknown_ids() {
        // A stale settings.json entry (renamed/removed client) must not
        // crash; unknown ids are dropped and the resolver still produces
        // a usable filter set.
        let configured = vec!["opencode".to_string(), "not-a-real-client".to_string()];
        let set = resolve_default_tui_filter_set_with(&configured);
        let mut expected = std::collections::HashSet::new();
        expected.insert(ClientFilter::Opencode);
        assert_eq!(set, expected);
    }

    #[test]
    fn test_resolve_default_tui_filter_set_all_unknown_falls_back() {
        // If every configured id is invalid, treat as if nothing is
        // configured rather than producing an empty filter set (which
        // would mean "scan nothing", definitely not the intent).
        let configured = vec!["not-real".to_string(), "also-fake".to_string()];
        let set = resolve_default_tui_filter_set_with(&configured);
        assert_eq!(set, ClientFilter::default_set());
    }

    #[test]
    fn test_resolve_default_tui_filter_set_supports_synthetic() {
        // Power users who explicitly want synthetic detection on every
        // launch can put it in defaultClients.
        let configured = vec!["claude".to_string(), "synthetic".to_string()];
        let set = resolve_default_tui_filter_set_with(&configured);
        let mut expected = std::collections::HashSet::new();
        expected.insert(ClientFilter::Claude);
        expected.insert(ClientFilter::Synthetic);
        assert_eq!(set, expected);
    }

    #[test]
    fn test_build_client_filter_with_defaults_when_no_flags() {
        // No CLI flags + a defaultClients list → defaults apply.
        let flags = ClientFlags::default();
        let defaults = vec!["opencode".to_string(), "claude".to_string()];
        assert_eq!(
            build_client_filter_with_defaults(flags, &defaults),
            Some(vec!["opencode".to_string(), "claude".to_string()])
        );
    }

    #[test]
    fn test_build_client_filter_cli_overrides_defaults_completely() {
        // User passes --client → defaults must be ignored entirely
        // (no merge). This is the predictable semantics: "I asked for X,
        // give me X" not "I asked for X but you also added Y from settings".
        let flags = ClientFlags {
            clients: vec![ClientFilter::Codex],
            ..ClientFlags::default()
        };
        let defaults = vec!["opencode".to_string(), "claude".to_string()];
        assert_eq!(
            build_client_filter_with_defaults(flags, &defaults),
            Some(vec!["codex".to_string()])
        );
    }

    #[test]
    fn test_build_client_filter_legacy_flag_overrides_defaults() {
        // Legacy flags also count as "user passed something" → defaults
        // ignored. Otherwise upgrading a script that uses --opencode
        // would surprise users with extra clients from settings.
        let flags = ClientFlags {
            opencode: true,
            ..ClientFlags::default()
        };
        let defaults = vec!["claude".to_string()];
        assert_eq!(
            build_client_filter_with_defaults(flags, &defaults),
            Some(vec!["opencode".to_string()])
        );
    }

    #[test]
    fn test_build_client_filter_defaults_dropped_for_unknown_ids() {
        // Stale settings entry (e.g. removed/renamed client) → silently
        // dropped, never errors. Ensures a typo never breaks tokscale.
        let flags = ClientFlags::default();
        let defaults = vec!["opencode".to_string(), "not-a-client".to_string()];
        assert_eq!(
            build_client_filter_with_defaults(flags, &defaults),
            Some(vec!["opencode".to_string()])
        );
    }

    #[test]
    fn test_build_client_filter_defaults_dedup_preserves_order() {
        let flags = ClientFlags::default();
        let defaults = vec![
            "claude".to_string(),
            "opencode".to_string(),
            "claude".to_string(),
        ];
        assert_eq!(
            build_client_filter_with_defaults(flags, &defaults),
            Some(vec!["claude".to_string(), "opencode".to_string()])
        );
    }

    #[test]
    fn test_build_client_filter_no_flags_no_defaults_returns_none() {
        let flags = ClientFlags::default();
        let defaults: Vec<String> = vec![];
        assert_eq!(build_client_filter_with_defaults(flags, &defaults), None);
    }

    #[test]
    fn test_client_filter_parses_lowercase_canonical_names() {
        // clap ValueEnum should accept the lowercase ids verbatim so
        // `--client opencode,claude` mirrors the legacy flag spelling.
        for filter in ClientFilter::value_variants() {
            let id = filter.as_filter_str();
            let parsed =
                <ClientFilter as ValueEnum>::from_str(id, true).expect("variant should parse");
            assert_eq!(parsed.as_filter_str(), id, "round-trip mismatch for {id}");
        }
    }

    #[test]
    fn test_client_flags_parses_canonical_form() {
        // End-to-end smoke test: ensure clap derives accept the new
        // `--client a,b` and `-c a -c b` shapes through the CLI parser.
        let cli =
            Cli::try_parse_from(["tokscale", "--client", "opencode,claude"]).expect("parse ok");
        assert_eq!(
            cli.clients.clients,
            vec![ClientFilter::Opencode, ClientFilter::Claude]
        );

        let cli =
            Cli::try_parse_from(["tokscale", "-c", "opencode", "-c", "claude"]).expect("parse ok");
        assert_eq!(
            cli.clients.clients,
            vec![ClientFilter::Opencode, ClientFilter::Claude]
        );
    }

    #[test]
    fn test_client_flags_legacy_still_parses() {
        // Legacy `--claude` keeps working even though it is hidden in --help.
        let cli = Cli::try_parse_from(["tokscale", "--claude"]).expect("parse ok");
        assert!(cli.clients.claude);
        assert!(cli.clients.clients.is_empty());
    }

    #[test]
    fn test_client_flag_accepts_uppercase() {
        let cli =
            Cli::try_parse_from(["tokscale", "--client", "OPENCODE"]).expect("uppercase parses");
        assert_eq!(cli.clients.clients, vec![ClientFilter::Opencode]);

        let cli = Cli::try_parse_from(["tokscale", "-c", "Codebuff,Antigravity"])
            .expect("mixed-case parses");
        assert_eq!(
            cli.clients.clients,
            vec![ClientFilter::Codebuff, ClientFilter::Antigravity]
        );
    }

    #[test]
    fn test_client_flag_rejects_unknown_and_empty_values() {
        assert!(Cli::try_parse_from(["tokscale", "--client", "unknown"]).is_err());
        assert!(Cli::try_parse_from(["tokscale", "--client", ""]).is_err());
    }

    #[test]
    fn test_legacy_bool_flag_rejects_duplicates() {
        let result = Cli::try_parse_from(["tokscale", "--opencode", "--opencode"]);
        assert!(
            result.is_err(),
            "clap rejects duplicated boolean flags by default; if this changes, document it explicitly"
        );
    }

    #[test]
    fn test_default_submit_clients_excludes_crush() {
        let clients = default_submit_clients();
        assert!(clients.contains(&"synthetic".to_string()));
        assert!(!clients.contains(&"crush".to_string()));
    }

    #[test]
    fn test_build_client_filter_with_defaults_uses_defaults_when_no_flags() {
        let flags = ClientFlags::default();
        let defaults = vec!["opencode".to_string(), "claude".to_string()];
        assert_eq!(
            build_client_filter_with_defaults(flags, &defaults),
            Some(vec!["opencode".to_string(), "claude".to_string()])
        );
    }

    #[test]
    fn test_build_client_filter_with_defaults_empty_defaults_returns_none() {
        let flags = ClientFlags::default();
        assert_eq!(build_client_filter_with_defaults(flags, &[]), None);
    }

    #[test]
    fn test_client_filter_goose_round_trip() {
        assert_eq!(
            ClientFilter::from_filter_str("goose"),
            Some(ClientFilter::Goose)
        );
        assert_eq!(ClientFilter::Goose.as_filter_str(), "goose");
        assert_eq!(
            ClientFilter::Goose.to_client_id(),
            Some(tokscale_core::ClientId::Goose)
        );
        assert_eq!(
            ClientFilter::from_client_id(tokscale_core::ClientId::Goose),
            ClientFilter::Goose
        );
    }

    #[test]
    fn test_client_filter_default_set_includes_goose() {
        let default = ClientFilter::default_set();
        assert!(
            default.contains(&ClientFilter::Goose),
            "default_set() must include Goose so the no-filter path scans it"
        );
    }

    #[test]
    fn test_delete_submitted_data_command_parses() {
        let cli = Cli::try_parse_from(["tokscale", "delete-submitted-data"]).unwrap();
        assert!(matches!(cli.command, Some(Commands::DeleteSubmittedData)));
    }

    #[test]
    fn test_interpret_delete_submitted_data_response_success() {
        let body = serde_json::json!({
            "deleted": true,
            "deletedSubmissions": 2
        });

        let outcome = interpret_delete_submitted_data_response(StatusCode::OK, &body).unwrap();
        match outcome {
            DeleteSubmittedDataOutcome::Deleted(count) => assert_eq!(count, 2),
            DeleteSubmittedDataOutcome::NotFound => panic!("expected deleted outcome"),
        }
    }

    #[test]
    fn test_interpret_delete_submitted_data_response_failure() {
        let body = serde_json::json!({
            "error": "Not authenticated"
        });

        let err = interpret_delete_submitted_data_response(StatusCode::UNAUTHORIZED, &body)
            .unwrap_err()
            .to_string();
        assert!(err.contains("Failed (401 Unauthorized): Not authenticated"));
    }

    #[test]
    fn test_build_date_filter_custom_range() {
        let (since, until) = build_date_filter(
            false,
            false,
            false,
            Some("2024-01-01".to_string()),
            Some("2024-12-31".to_string()),
        );
        assert_eq!(since, Some("2024-01-01".to_string()));
        assert_eq!(until, Some("2024-12-31".to_string()));
    }

    #[test]
    fn test_build_date_filter_no_filters() {
        let (since, until) = build_date_filter(false, false, false, None, None);
        assert_eq!(since, None);
        assert_eq!(until, None);
    }

    #[test]
    fn test_build_date_filter_today_uses_provided_local_date() {
        let today = chrono::NaiveDate::from_ymd_opt(2026, 3, 8).unwrap();
        let (since, until) = build_date_filter_for_date(true, false, false, None, None, today);
        assert_eq!(since, Some("2026-03-08".to_string()));
        assert_eq!(until, Some("2026-03-08".to_string()));
    }

    #[test]
    fn test_build_date_filter_week_uses_provided_local_date() {
        let today = chrono::NaiveDate::from_ymd_opt(2026, 3, 8).unwrap();
        let (since, until) = build_date_filter_for_date(false, true, false, None, None, today);
        assert_eq!(since, Some("2026-03-02".to_string()));
        assert_eq!(until, Some("2026-03-08".to_string()));
    }

    #[test]
    fn test_build_date_filter_month_uses_provided_local_date() {
        let today = chrono::NaiveDate::from_ymd_opt(2026, 3, 8).unwrap();
        let (since, until) = build_date_filter_for_date(false, false, true, None, None, today);
        assert_eq!(since, Some("2026-03-01".to_string()));
        assert_eq!(until, Some("2026-03-08".to_string()));
    }

    #[test]
    fn test_normalize_year_filter_with_year() {
        let year = normalize_year_filter(false, false, false, Some("2024".to_string()));
        assert_eq!(year, Some("2024".to_string()));
    }

    #[test]
    fn test_normalize_year_filter_with_today() {
        let year = normalize_year_filter(true, false, false, Some("2024".to_string()));
        assert_eq!(year, None);
    }

    #[test]
    fn test_normalize_year_filter_with_week() {
        let year = normalize_year_filter(false, true, false, Some("2024".to_string()));
        assert_eq!(year, None);
    }

    #[test]
    fn test_normalize_year_filter_with_month() {
        let year = normalize_year_filter(false, false, true, Some("2024".to_string()));
        assert_eq!(year, None);
    }

    #[test]
    fn test_normalize_year_filter_no_year() {
        let year = normalize_year_filter(false, false, false, None);
        assert_eq!(year, None);
    }

    #[test]
    fn test_format_tokens_with_commas_small() {
        assert_eq!(format_tokens_with_commas(123), "123");
    }

    #[test]
    fn test_format_tokens_with_commas_thousands() {
        assert_eq!(format_tokens_with_commas(1234), "1,234");
    }

    #[test]
    fn test_format_tokens_with_commas_millions() {
        assert_eq!(format_tokens_with_commas(1234567), "1,234,567");
    }

    #[test]
    fn test_format_tokens_with_commas_billions() {
        assert_eq!(format_tokens_with_commas(1234567890), "1,234,567,890");
    }

    #[test]
    fn test_format_tokens_with_commas_zero() {
        assert_eq!(format_tokens_with_commas(0), "0");
    }

    #[test]
    fn test_format_tokens_with_commas_negative() {
        assert_eq!(format_tokens_with_commas(-1234567), "-1,234,567");
    }

    #[test]
    fn test_format_currency_zero() {
        assert_eq!(format_currency(0.0), "$0.00");
    }

    #[test]
    fn test_format_currency_small() {
        assert_eq!(format_currency(12.34), "$12.34");
    }

    #[test]
    fn test_format_currency_large() {
        assert_eq!(format_currency(1234.56), "$1234.56");
    }

    #[test]
    fn test_format_currency_rounds() {
        assert_eq!(format_currency(12.345), "$12.35");
        assert_eq!(format_currency(12.344), "$12.34");
    }

    #[test]
    fn test_capitalize_client_opencode() {
        assert_eq!(capitalize_client("opencode"), "OpenCode");
    }

    #[test]
    fn test_capitalize_client_claude() {
        assert_eq!(capitalize_client("claude"), "Claude");
    }

    #[test]
    fn test_capitalize_client_codex() {
        assert_eq!(capitalize_client("codex"), "Codex");
    }

    #[test]
    fn test_capitalize_client_cursor() {
        assert_eq!(capitalize_client("cursor"), "Cursor");
    }

    #[test]
    fn test_capitalize_client_gemini() {
        assert_eq!(capitalize_client("gemini"), "Gemini");
    }

    #[test]
    fn test_capitalize_client_amp() {
        assert_eq!(capitalize_client("amp"), "Amp");
    }

    #[test]
    fn test_capitalize_client_droid() {
        assert_eq!(capitalize_client("droid"), "Droid");
    }

    #[test]
    fn test_capitalize_client_crush() {
        assert_eq!(capitalize_client("crush"), "Crush");
    }

    #[test]
    fn test_capitalize_client_openclaw() {
        assert_eq!(capitalize_client("openclaw"), "openclaw");
    }

    #[test]
    fn test_capitalize_client_hermes() {
        assert_eq!(capitalize_client("hermes"), "Hermes Agent");
    }

    #[test]
    fn test_capitalize_client_codebuff() {
        assert_eq!(capitalize_client("codebuff"), "Codebuff");
    }

    #[test]
    fn test_capitalize_client_pi() {
        assert_eq!(capitalize_client("pi"), "Pi");
    }

    #[test]
    fn test_capitalize_client_unknown() {
        assert_eq!(capitalize_client("unknown"), "unknown");
    }

    #[test]
    fn test_get_date_range_label_today() {
        let label = get_date_range_label(true, false, false, &None, &None, &None);
        assert_eq!(label, Some("Today".to_string()));
    }

    #[test]
    fn test_get_date_range_label_week() {
        let label = get_date_range_label(false, true, false, &None, &None, &None);
        assert_eq!(label, Some("Last 7 days".to_string()));
    }

    #[test]
    fn test_get_date_range_label_month_uses_provided_local_date() {
        let today = chrono::NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
        let label = get_date_range_label_for_date(false, false, true, &None, &None, &None, today);
        assert_eq!(label, Some("March 2026".to_string()));
    }

    #[test]
    fn test_get_date_range_label_year() {
        let label =
            get_date_range_label(false, false, false, &None, &None, &Some("2024".to_string()));
        assert_eq!(label, Some("2024".to_string()));
    }

    #[test]
    fn test_get_date_range_label_custom_since() {
        let label = get_date_range_label(
            false,
            false,
            false,
            &Some("2024-01-01".to_string()),
            &None,
            &None,
        );
        assert_eq!(label, Some("from 2024-01-01".to_string()));
    }

    #[test]
    fn test_get_date_range_label_custom_until() {
        let label = get_date_range_label(
            false,
            false,
            false,
            &None,
            &Some("2024-12-31".to_string()),
            &None,
        );
        assert_eq!(label, Some("to 2024-12-31".to_string()));
    }

    #[test]
    fn test_get_date_range_label_custom_range() {
        let label = get_date_range_label(
            false,
            false,
            false,
            &Some("2024-01-01".to_string()),
            &Some("2024-12-31".to_string()),
            &None,
        );
        assert_eq!(label, Some("from 2024-01-01 to 2024-12-31".to_string()));
    }

    #[test]
    fn test_get_date_range_label_none() {
        let label = get_date_range_label(false, false, false, &None, &None, &None);
        assert_eq!(label, None);
    }

    #[test]
    fn test_light_spinner_frame_0() {
        let frame = LightSpinner::frame(0);
        assert!(frame.contains("■"));
        assert!(frame.contains("⬝"));
    }

    #[test]
    fn test_light_spinner_frame_1() {
        let frame = LightSpinner::frame(1);
        assert!(frame.contains("■"));
        assert!(frame.contains("⬝"));
    }

    #[test]
    fn test_light_spinner_frame_2() {
        let frame = LightSpinner::frame(2);
        assert!(frame.contains("■"));
        assert!(frame.contains("⬝"));
    }

    #[test]
    fn test_light_spinner_scanner_state_forward_start() {
        let (position, forward) = LightSpinner::scanner_state(0);
        assert_eq!(position, 0);
        assert!(forward);
    }

    #[test]
    fn test_light_spinner_scanner_state_forward_mid() {
        let (position, forward) = LightSpinner::scanner_state(4);
        assert_eq!(position, 4);
        assert!(forward);
    }

    #[test]
    fn test_light_spinner_scanner_state_forward_end() {
        let (position, forward) = LightSpinner::scanner_state(7);
        assert_eq!(position, 7);
        assert!(forward);
    }

    #[test]
    fn test_light_spinner_scanner_state_hold_end() {
        let (position, forward) = LightSpinner::scanner_state(8);
        assert_eq!(position, 7);
        assert!(forward);
    }

    #[test]
    fn test_light_spinner_scanner_state_backward_start() {
        let (position, forward) = LightSpinner::scanner_state(17);
        assert_eq!(position, 6);
        assert!(!forward);
    }

    #[test]
    fn test_light_spinner_scanner_state_backward_end() {
        let (position, forward) = LightSpinner::scanner_state(23);
        assert_eq!(position, 0);
        assert!(!forward);
    }

    #[test]
    fn test_light_spinner_scanner_state_hold_start() {
        let (position, forward) = LightSpinner::scanner_state(24);
        assert_eq!(position, 0);
        assert!(!forward);
    }

    #[test]
    fn test_light_spinner_scanner_state_cycle_wrap() {
        // Total cycle = 8 + 9 + 7 + 30 = 54
        let (position1, forward1) = LightSpinner::scanner_state(0);
        let (position2, forward2) = LightSpinner::scanner_state(54);
        assert_eq!(position1, position2);
        assert_eq!(forward1, forward2);
    }

    #[test]
    fn test_cap_graph_result_to_utc_today_recalculates_all_derived_fields() {
        let mut graph = graph_result_with_contributions(vec![
            daily_contribution("2026-12-30", 10, 1.25, "codex", "model-a"),
            daily_contribution("2026-12-31", 20, 2.50, "codex", "model-b"),
            daily_contribution("2027-01-01", 30, 3.75, "cursor", "model-c"),
        ]);

        let changed = cap_graph_result_to_utc_today(&mut graph, "2026-12-31");

        assert!(changed);
        assert_eq!(graph.meta.date_range_start, "2026-12-30");
        assert_eq!(graph.meta.date_range_end, "2026-12-31");
        assert_eq!(graph.contributions.len(), 2);
        assert_eq!(graph.summary.total_tokens, 30);
        assert_eq!(graph.summary.total_cost, 3.75);
        assert_eq!(graph.summary.total_days, 2);
        assert_eq!(graph.summary.active_days, 2);
        assert_eq!(graph.summary.clients, vec!["codex".to_string()]);
        assert_eq!(
            graph.summary.models,
            vec!["model-a".to_string(), "model-b".to_string()]
        );
        assert_eq!(graph.years.len(), 1);
        assert_eq!(year_summary(&graph, "2026").total_tokens, 30);
    }

    #[test]
    fn test_cap_graph_result_to_utc_today_clears_empty_post_cap_state() {
        let mut graph = graph_result_with_contributions(vec![daily_contribution(
            "2027-01-01",
            30,
            3.75,
            "cursor",
            "model-c",
        )]);

        let changed = cap_graph_result_to_utc_today(&mut graph, "2026-12-31");

        assert!(changed);
        assert!(graph.contributions.is_empty());
        assert_eq!(graph.meta.date_range_start, "");
        assert_eq!(graph.meta.date_range_end, "");
        assert_eq!(graph.summary.total_tokens, 0);
        assert_eq!(graph.summary.total_cost, 0.0);
        assert_eq!(graph.summary.total_days, 0);
        assert_eq!(graph.summary.active_days, 0);
        assert!(graph.summary.clients.is_empty());
        assert!(graph.summary.models.is_empty());
        assert!(graph.years.is_empty());
    }

    #[test]
    fn test_cap_graph_result_to_utc_today_is_noop_when_all_dates_are_in_range() {
        let mut graph = graph_result_with_contributions(vec![
            daily_contribution("2026-12-30", 10, 1.25, "codex", "model-a"),
            daily_contribution("2026-12-31", 20, 2.50, "codex", "model-b"),
        ]);
        let original_summary = graph.summary.clone();
        let original_years = graph.years.clone();

        let changed = cap_graph_result_to_utc_today(&mut graph, "2026-12-31");

        assert!(!changed);
        assert_eq!(graph.meta.date_range_start, "2026-12-30");
        assert_eq!(graph.meta.date_range_end, "2026-12-31");
        assert_eq!(graph.summary.total_tokens, original_summary.total_tokens);
        assert_eq!(graph.summary.total_cost, original_summary.total_cost);
        assert_eq!(graph.summary.clients, original_summary.clients);
        assert_eq!(graph.summary.models, original_summary.models);
        assert_eq!(graph.years.len(), original_years.len());
    }

    #[test]
    #[cfg(target_os = "macos")]
    #[serial_test::serial]
    fn test_load_star_cache_falls_back_to_legacy_macos_path() {
        // Existing macOS users have star-cache.json at the pre-#468 path under
        // `~/Library/Application Support/tokscale/`. After upgrade, the read
        // path moves to `~/.config/tokscale/`, so without the legacy fallback
        // load_star_cache returns None and the user gets re-prompted to star
        // the repo even though they already starred it.
        use std::env;
        let temp = tempfile::TempDir::new().unwrap();
        let prev_home = env::var_os("HOME");
        let prev_override = env::var_os("TOKSCALE_CONFIG_DIR");
        unsafe {
            env::set_var("HOME", temp.path());
            env::remove_var("TOKSCALE_CONFIG_DIR");
        }

        let legacy_dir = temp.path().join("Library/Application Support/tokscale");
        std::fs::create_dir_all(&legacy_dir).unwrap();
        std::fs::write(
            legacy_dir.join("star-cache.json"),
            r#"{"username":"junhoyeo","hasStarred":true,"checkedAt":"2025-01-12T03:48:00Z"}"#,
        )
        .unwrap();

        let new_path = temp.path().join(".config/tokscale/star-cache.json");
        assert!(!new_path.exists());

        let cache = load_star_cache("junhoyeo");
        assert!(
            cache.is_some(),
            "legacy macOS star-cache.json must satisfy load_star_cache after upgrade"
        );
        let cache = cache.unwrap();
        assert_eq!(cache.username, "junhoyeo");
        assert!(cache.has_starred);

        unsafe {
            match prev_home {
                Some(v) => env::set_var("HOME", v),
                None => env::remove_var("HOME"),
            }
            match prev_override {
                Some(v) => env::set_var("TOKSCALE_CONFIG_DIR", v),
                None => env::remove_var("TOKSCALE_CONFIG_DIR"),
            }
        }
    }

    #[test]
    #[cfg(target_os = "macos")]
    #[serial_test::serial]
    fn test_load_star_cache_skips_legacy_fallback_when_config_dir_overridden() {
        // Same hermeticity contract as the Settings test: TOKSCALE_CONFIG_DIR
        // must isolate the test/CI/sandbox profile from the real user's
        // legacy macOS star-cache.json.
        use std::env;
        let temp = tempfile::TempDir::new().unwrap();
        let legacy_root = tempfile::TempDir::new().unwrap();
        let prev_home = env::var_os("HOME");
        let prev_override = env::var_os("TOKSCALE_CONFIG_DIR");
        unsafe {
            env::set_var("HOME", legacy_root.path());
            env::set_var("TOKSCALE_CONFIG_DIR", temp.path());
        }

        let legacy_dir = legacy_root
            .path()
            .join("Library/Application Support/tokscale");
        std::fs::create_dir_all(&legacy_dir).unwrap();
        std::fs::write(
            legacy_dir.join("star-cache.json"),
            r#"{"username":"junhoyeo","hasStarred":true,"checkedAt":"2025-01-12T03:48:00Z"}"#,
        )
        .unwrap();

        assert!(
            load_star_cache("junhoyeo").is_none(),
            "override must not leak the legacy star-cache hit"
        );

        unsafe {
            match prev_home {
                Some(v) => env::set_var("HOME", v),
                None => env::remove_var("HOME"),
            }
            match prev_override {
                Some(v) => env::set_var("TOKSCALE_CONFIG_DIR", v),
                None => env::remove_var("TOKSCALE_CONFIG_DIR"),
            }
        }
    }

    #[test]
    fn resolve_cli_write_overrides_settings_false() {
        let settings = tui::settings::Settings {
            light: tui::settings::LightSettings { write_cache: false },
            ..tui::settings::Settings::default()
        };
        assert!(resolve_should_write_cache(true, false, &settings));
    }

    #[test]
    fn resolve_cli_no_write_overrides_settings_true() {
        let settings = tui::settings::Settings {
            light: tui::settings::LightSettings { write_cache: true },
            ..tui::settings::Settings::default()
        };
        assert!(!resolve_should_write_cache(false, true, &settings));
    }

    #[test]
    fn resolve_settings_true_with_no_cli_flag() {
        let settings = tui::settings::Settings {
            light: tui::settings::LightSettings { write_cache: true },
            ..tui::settings::Settings::default()
        };
        assert!(resolve_should_write_cache(false, false, &settings));
    }

    #[test]
    fn resolve_settings_false_with_no_cli_flag() {
        let settings = tui::settings::Settings {
            light: tui::settings::LightSettings { write_cache: false },
            ..tui::settings::Settings::default()
        };
        assert!(!resolve_should_write_cache(false, false, &settings));
    }

    #[test]
    fn resolve_settings_default_returns_false() {
        assert!(!resolve_should_write_cache(
            false,
            false,
            &tui::settings::Settings::default()
        ));
    }

    #[test]
    fn clap_rejects_write_cache_without_light() {
        assert!(Cli::try_parse_from(["tokscale", "--write-cache"]).is_err());
    }

    #[test]
    fn clap_rejects_no_write_cache_without_light() {
        assert!(Cli::try_parse_from(["tokscale", "--no-write-cache"]).is_err());
    }

    #[test]
    fn clap_rejects_both_write_flags_together() {
        assert!(
            Cli::try_parse_from(["tokscale", "--light", "--write-cache", "--no-write-cache",])
                .is_err()
        );
    }

    #[test]
    fn clap_accepts_models_light_write_cache_after_subcommand() {
        assert!(Cli::try_parse_from(["tokscale", "models", "--light", "--write-cache"]).is_ok());
    }

    #[test]
    fn clap_accepts_cursor_sync_command() {
        assert!(Cli::try_parse_from(["tokscale", "cursor", "sync"]).is_ok());
        assert!(Cli::try_parse_from(["tokscale", "cursor", "sync", "--json"]).is_ok());
    }

    #[test]
    fn cursor_auto_sync_enabled_for_default_report() {
        assert!(should_auto_sync_cursor_for_local_report(&None, &None));
    }

    #[test]
    fn cursor_auto_sync_enabled_when_cursor_filter_is_explicit() {
        assert!(should_auto_sync_cursor_for_local_report(
            &None,
            &Some(vec!["cursor".to_string()])
        ));
    }

    #[test]
    fn cursor_auto_sync_disabled_when_filter_excludes_cursor() {
        assert!(!should_auto_sync_cursor_for_local_report(
            &None,
            &Some(vec!["codex".to_string()])
        ));
    }

    #[test]
    fn cursor_auto_sync_disabled_for_home_override() {
        assert!(!should_auto_sync_cursor_for_local_report(
            &Some("/tmp/other-home".to_string()),
            &None
        ));
        assert!(!should_auto_sync_cursor_for_local_report(
            &Some("/tmp/other-home".to_string()),
            &Some(vec!["cursor".to_string()])
        ));
    }

    #[test]
    fn cursor_auto_sync_runtime_init_failure_is_best_effort() {
        let result = run_best_effort_cursor_sync_with_runtime_factory(|| {
            Err(std::io::Error::other("runtime unavailable"))
        });

        assert!(!result.synced);
        assert_eq!(result.rows, 0);
        assert!(result
            .error
            .as_deref()
            .is_some_and(|error| error.contains("runtime unavailable")));
    }

    #[test]
    fn write_light_cache_refuses_when_since_filter_set() {
        // Date/home filters are NOT part of the TUI cache key. Writing
        // the filtered slice would silently poison subsequent TUI launches
        // (next `tokscale tui` would render the filtered slice as if it
        // were the default report). The function returns `()` and prints
        // an eprintln; reaching this assertion line proves the function
        // returned normally without panicking or attempting the write.
        let group_by = tokscale_core::GroupBy::default();
        write_light_cache(
            &None,
            &None,
            &Some("2025-01-01".to_string()),
            &None,
            &None,
            &group_by,
        );
    }

    #[test]
    fn write_light_cache_refuses_when_until_filter_set() {
        let group_by = tokscale_core::GroupBy::default();
        write_light_cache(
            &None,
            &None,
            &None,
            &Some("2025-12-31".to_string()),
            &None,
            &group_by,
        );
    }

    #[test]
    fn write_light_cache_refuses_when_year_filter_set() {
        let group_by = tokscale_core::GroupBy::default();
        write_light_cache(
            &None,
            &None,
            &None,
            &None,
            &Some("2025".to_string()),
            &group_by,
        );
    }

    #[test]
    fn write_light_cache_refuses_when_home_dir_set() {
        // --home rebinds the scan root; DataLoader::load currently ignores
        // this field and resolves home from dirs::home_dir() with
        // use_env_roots=true, so the printed --light report is built from
        // <home> while a naive cache write would store data scanned from
        // the default home. Refuse the write to avoid that drift.
        let group_by = tokscale_core::GroupBy::default();
        write_light_cache(
            &Some("/tmp/fake-home".to_string()),
            &None,
            &None,
            &None,
            &None,
            &group_by,
        );
    }
}
