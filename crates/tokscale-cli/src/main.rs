mod antigravity;
mod auth;
mod claude_diagnostics;
mod commands;
mod cursor;
mod device;
mod paths;
mod trae;
mod tui;
mod warp;

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
        help = "Grouping strategy for --light and --json output: model, client,model, client,provider,model, workspace,model, session,model, client,session,model"
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
            help = "Grouping strategy for --light and --json output: model, client,model, client,provider,model, workspace,model, session,model, client,session,model"
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
        #[arg(help = "Model ID to look up, or `list-overrides`")]
        model_id: String,
        #[arg(long, help = "Output as JSON")]
        json: bool,
        #[arg(
            long,
            help = "Force specific pricing source (custom, litellm, openrouter, or models.dev)"
        )]
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
    Login {
        #[arg(
            long,
            help = "Save an existing Tokscale API token without browser auth"
        )]
        token: Option<String>,
    },
    #[command(about = "Logout from Tokscale")]
    Logout,
    #[command(about = "Show current logged in user")]
    Whoami,
    #[command(about = "Display saved API token as QR code")]
    Qr {
        #[arg(long, help = "Skip the on-screen warning + confirmation prompt")]
        yes: bool,
    },
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
        #[arg(
            long = "clients",
            help = "Show Top Clients instead of Top OpenCode Agents"
        )]
        show_clients: bool,
        #[arg(long, help = "Disable pinning of Sisyphus agents in rankings")]
        disable_pinned: bool,
        #[arg(long, help = "Disable loading spinner (for scripting)")]
        no_spinner: bool,
    },
    #[command(about = "Show subscription usage and quota for AI providers")]
    Usage {
        #[arg(long, help = "Output as JSON")]
        json: bool,
        #[arg(long, help = "Light terminal output (no TUI)")]
        light: bool,
    },
    #[command(about = "Codex account integration commands")]
    Codex {
        #[command(subcommand)]
        subcommand: CodexSubcommand,
    },
    #[command(about = "Cursor API cache integration commands")]
    Cursor {
        #[command(subcommand)]
        subcommand: CursorSubcommand,
    },
    #[command(about = "Antigravity integration commands")]
    Antigravity {
        #[command(subcommand)]
        subcommand: AntigravitySubcommand,
    },
    #[command(about = "Trae IDE integration commands")]
    Trae {
        #[command(subcommand)]
        subcommand: TraeSubcommand,
    },
    #[command(about = "Warp/Oz aggregate usage integration commands")]
    Warp {
        #[command(subcommand)]
        subcommand: WarpSubcommand,
    },
    #[command(about = "Delete all submitted usage data from the server")]
    DeleteSubmittedData,
    #[command(
        about = "Show session time metrics (usage time, longest continuous, max concurrent)"
    )]
    TimeMetrics {
        #[arg(long)]
        json: bool,
        #[command(flatten)]
        clients: ClientFlags,
        #[command(flatten)]
        date: DateRangeFlags,
        #[arg(long, help = "Disable spinner")]
        no_spinner: bool,
    },
    #[command(about = "Warm TUI cache in background (internal)", hide = true)]
    WarmTuiCache,
    #[command(about = "Task-attributed usage report")]
    Report {
        #[arg(long, help = "Output as JSON")]
        json: bool,
        #[arg(long, help = "Filter by workspace path")]
        workspace: Option<String>,
        #[arg(long, help = "Filter by client (opencode, claude, codex, etc.)")]
        client: Option<String>,
        #[command(flatten)]
        date: DateRangeFlags,
        #[arg(long, help = "Skip LLM summarization (show raw data only)")]
        no_summarize: bool,
        #[arg(
            long,
            default_value = "apple-fm",
            help = "Summarizer backend: apple-fm, claude, codex, gemini, kiro"
        )]
        summarizer: String,
        #[arg(long, help = "Reset all summaries and re-summarize from scratch")]
        rebuild: bool,
    },
}

#[derive(Subcommand)]
enum CursorSubcommand {
    #[command(about = "Login to Cursor with a browser session token")]
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
    #[command(about = "Sync Cursor API usage into cursor-cache/usage*.csv")]
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
enum CodexSubcommand {
    #[command(about = "Import the current Codex OAuth credentials as a saved account")]
    Import {
        #[arg(long, help = "Label for this Codex account (e.g., work, personal)")]
        name: Option<String>,
    },
    #[command(about = "List saved Codex accounts")]
    Accounts {
        #[arg(long, help = "Output as JSON")]
        json: bool,
    },
    #[command(about = "Switch active Codex account and write Codex auth.json")]
    Switch {
        #[arg(help = "Account label or id")]
        name: String,
    },
    #[command(about = "Remove a saved Codex account")]
    Remove {
        #[arg(help = "Account label or id")]
        name: String,
    },
    #[command(about = "Check Codex subscription usage for an account")]
    Status {
        #[arg(long, help = "Account label or id")]
        name: Option<String>,
        #[arg(long, help = "Output as JSON")]
        json: bool,
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

#[derive(Subcommand)]
enum TraeSubcommand {
    #[command(about = "Authenticate Trae — auto-detect from desktop client or paste JWT")]
    Login {
        #[arg(long, help = "Paste access token directly (for manual fallback)")]
        manual: bool,
        #[arg(long, help = "Target Trae variant (solo, ide)")]
        variant: Option<String>,
    },
    #[command(about = "Remove cached Trae credentials")]
    Logout {
        #[arg(long, help = "Target Trae variant (solo, ide)")]
        variant: Option<String>,
    },
    #[command(about = "Show Trae authentication status")]
    Status {
        #[arg(long, help = "Output as JSON")]
        json: bool,
    },
    #[command(about = "Sync Trae usage data into local cache")]
    Sync {
        #[arg(long, help = "Number of days to sync (default: 30)")]
        since: Option<i64>,
        #[arg(long, help = "Include auxiliary usage types (not just main chat)")]
        include_aux: bool,
    },
}

#[derive(Subcommand)]
enum WarpSubcommand {
    #[command(about = "Save Warp GraphQL authentication for aggregate usage sync")]
    Login {
        #[arg(long, help = "Warp bearer token or cookie header value")]
        token: Option<String>,
        #[arg(
            long,
            help = "Treat token as a Cookie header instead of a bearer token"
        )]
        cookie: bool,
    },
    #[command(about = "Remove cached Warp credentials")]
    Logout {
        #[arg(long, help = "Also delete cached Warp aggregate usage")]
        purge_cache: bool,
    },
    #[command(about = "Show Warp aggregate sync status")]
    Status {
        #[arg(long, help = "Output as JSON")]
        json: bool,
    },
    #[command(about = "Sync Warp aggregate usage into local cache")]
    Sync {
        #[arg(long, help = "Output as JSON")]
        json: bool,
    },
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
            let clients = build_client_filter(clients, &cli.home);
            if json || light || !can_use_tui {
                run_models_report(
                    json,
                    cli.home.clone(),
                    clients,
                    &date,
                    benchmark,
                    no_spinner || !can_use_tui,
                    group_by,
                    write_cache,
                    no_write_cache,
                )
            } else {
                let (since, until) = build_date_filter(&date);
                let year = normalize_year_filter(&date);
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
            let clients = build_client_filter(clients, &cli.home);
            if json || light || !can_use_tui {
                run_monthly_report(
                    json,
                    cli.home.clone(),
                    clients,
                    &date,
                    benchmark,
                    no_spinner || !can_use_tui,
                )
            } else {
                let (since, until) = build_date_filter(&date);
                let year = normalize_year_filter(&date);
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
            let clients = build_client_filter(clients, &cli.home);
            if json || light || !can_use_tui {
                run_hourly_report(
                    json,
                    cli.home.clone(),
                    clients,
                    &date,
                    benchmark,
                    no_spinner || !can_use_tui,
                )
            } else {
                let (since, until) = build_date_filter(&date);
                let year = normalize_year_filter(&date);
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
        Some(Commands::Clients { json }) => run_clients_command(json, cli.home.clone()),
        Some(Commands::Login { token }) => {
            reject_unsupported_home_override(&cli.home, "login")?;
            run_login_command(token)
        }
        Some(Commands::Logout) => {
            reject_unsupported_home_override(&cli.home, "logout")?;
            run_logout_command()
        }
        Some(Commands::Whoami) => {
            reject_unsupported_home_override(&cli.home, "whoami")?;
            run_whoami_command()
        }
        Some(Commands::Qr { yes }) => {
            reject_unsupported_home_override(&cli.home, "qr")?;
            run_qr_command(yes)
        }
        Some(Commands::Graph {
            output,
            clients,
            date,
            benchmark,
            no_spinner,
        }) => {
            let (since, until) = build_date_filter(&date);
            let year = normalize_year_filter(&date);
            let clients = build_client_filter(clients, &cli.home);
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
            let (since, until) = build_date_filter(&date);
            let year = normalize_year_filter(&date);
            let clients = build_client_filter(clients, &cli.home);
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
            let (since, until) = build_date_filter(&date);
            let year = normalize_year_filter(&date);
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
            show_clients,
            disable_pinned,
            no_spinner: _,
        }) => {
            reject_unsupported_home_override(&cli.home, "wrapped")?;
            let client_filter = build_client_filter(client_flags, &cli.home);
            run_wrapped_command(
                output,
                year,
                client_filter,
                short,
                agents,
                show_clients,
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
        Some(Commands::Usage { json, light }) => {
            reject_unsupported_home_override(&cli.home, "usage")?;
            commands::usage::run(json, light)
        }
        Some(Commands::Codex { subcommand }) => {
            reject_unsupported_home_override(&cli.home, "codex")?;
            run_codex_command(subcommand)
        }
        Some(Commands::Trae { subcommand }) => {
            reject_unsupported_home_override(&cli.home, "trae")?;
            run_trae_command(subcommand)
        }
        Some(Commands::Warp { subcommand }) => {
            reject_unsupported_home_override(&cli.home, "warp")?;
            run_warp_command(subcommand)
        }
        Some(Commands::DeleteSubmittedData) => {
            reject_unsupported_home_override(&cli.home, "delete-submitted-data")?;
            run_delete_data_command()
        }
        Some(Commands::TimeMetrics {
            json,
            clients,
            date,
            no_spinner,
        }) => {
            let (since, until) = build_date_filter(&date);
            let year = normalize_year_filter(&date);
            let clients = build_client_filter(clients, &cli.home);
            run_time_metrics_report(
                json,
                cli.home.clone(),
                clients,
                since,
                until,
                year,
                no_spinner,
            )
        }
        Some(Commands::WarmTuiCache) => run_warm_tui_cache(),
        Some(Commands::Report {
            json,
            workspace,
            client,
            date,
            no_summarize,
            summarizer,
            rebuild,
        }) => {
            let today = date.today;
            let week = date.week;
            let month = date.month;
            let (since, until) = build_date_filter(&date);
            commands::report::run_report(commands::report::ReportOptions {
                json,
                since,
                until,
                workspace,
                client,
                no_summarize,
                summarizer,
                rebuild,
                home_dir: cli.home.clone(),
                scanner_settings: tui::settings::load_scanner_settings(),
                today,
                week,
                month,
            })
        }
        None => {
            let clients = build_client_filter(cli.clients, &cli.home);
            let group_by: tokscale_core::GroupBy = cli.group_by.parse().unwrap_or_else(|e| {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            });

            if cli.json {
                run_models_report(
                    cli.json,
                    cli.home.clone(),
                    clients,
                    &cli.date,
                    cli.benchmark,
                    cli.no_spinner || cli.json,
                    group_by,
                    cli.write_cache,
                    cli.no_write_cache,
                )
            } else if cli.light || !can_use_tui {
                run_models_report(
                    false,
                    cli.home.clone(),
                    clients,
                    &cli.date,
                    cli.benchmark,
                    cli.no_spinner || !can_use_tui,
                    group_by,
                    cli.write_cache,
                    cli.no_write_cache,
                )
            } else {
                let (since, until) = build_date_filter(&cli.date);
                let year = normalize_year_filter(&cli.date);
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
    Zed,
    Kiro,
    #[value(name = "trae")]
    Trae,
    Warp,
    Cline,
    Gjc,
    Grok,
    Jcode,
    Commandcode,
    Micode,
    #[value(name = "antigravity-cli")]
    AntigravityCli,
    Junie,
    Zcode,
    Opencodereview,
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
            Self::Zed => "zed",
            Self::Kiro => "kiro",
            Self::Trae => "trae",
            Self::Warp => "warp",
            Self::Cline => "cline",
            Self::Gjc => "gjc",
            Self::Grok => "grok",
            Self::Jcode => "jcode",
            Self::Commandcode => "commandcode",
            Self::Micode => "micode",
            Self::AntigravityCli => "antigravity-cli",
            Self::Junie => "junie",
            Self::Zcode => "zcode",
            Self::Opencodereview => "opencodereview",
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
            Self::Zed => Some(ClientId::Zed),
            Self::Kiro => Some(ClientId::Kiro),
            Self::Trae => Some(ClientId::Trae),
            Self::Warp => Some(ClientId::Warp),
            Self::Cline => Some(ClientId::Cline),
            Self::Gjc => Some(ClientId::Gjc),
            Self::Grok => Some(ClientId::Grok),
            Self::Jcode => Some(ClientId::Jcode),
            Self::Commandcode => Some(ClientId::CommandCode),
            Self::Micode => Some(ClientId::MiMoCode),
            Self::AntigravityCli => Some(ClientId::AntigravityCli),
            Self::Junie => Some(ClientId::Junie),
            Self::Zcode => Some(ClientId::Zcode),
            Self::Opencodereview => Some(ClientId::OpenCodeReview),
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
            ClientId::Zed => Self::Zed,
            ClientId::Kiro => Self::Kiro,
            ClientId::Trae => Self::Trae,
            ClientId::Warp => Self::Warp,
            ClientId::Cline => Self::Cline,
            ClientId::Gjc => Self::Gjc,
            ClientId::Grok => Self::Grok,
            ClientId::Jcode => Self::Jcode,
            ClientId::CommandCode => Self::Commandcode,
            ClientId::MiMoCode => Self::Micode,
            ClientId::AntigravityCli => Self::AntigravityCli,
            ClientId::Junie => Self::Junie,
            ClientId::Zcode => Self::Zcode,
            ClientId::OpenCodeReview => Self::Opencodereview,
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
        id = "client_filter",
        long = "client",
        short = 'c',
        value_name = "CLIENTS",
        value_enum,
        value_delimiter = ',',
        action = clap::ArgAction::Append,
        ignore_case = true,
        help = "Filter by client(s). Repeatable or comma-separated (e.g. -c opencode,claude)."
    )]
    pub clients: Vec<ClientFilter>,
}

#[derive(Args, Clone, Debug, Default)]
pub struct DateRangeFlags {
    #[arg(
        long,
        help = "Show only today's usage",
        conflicts_with_all = ["yesterday", "week", "month", "since", "until", "year"]
    )]
    pub today: bool,
    #[arg(
        long,
        help = "Show only yesterday's usage",
        conflicts_with_all = ["week", "month", "since", "until", "year"]
    )]
    pub yesterday: bool,
    #[arg(
        long,
        help = "Show last 7 days",
        conflicts_with_all = ["month", "since", "until", "year"]
    )]
    pub week: bool,
    #[arg(
        long,
        help = "Show current month",
        conflicts_with_all = ["since", "until", "year"]
    )]
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
/// 2. If step 1 produced nothing, fall back to user-configured
///    `defaultClients` from `~/.config/tokscale/settings.json` when present.
/// 3. Deduplicate while preserving first-seen order.
///
/// Returns `None` when no filters are active *and* no defaults configured
/// so the caller can scan all clients.
fn build_client_filter(flags: ClientFlags, home_dir: &Option<String>) -> Option<Vec<String>> {
    let defaults = tui::settings::load_default_clients_for_home(home_dir);
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

    // Defaults only apply when the user passed no canonical `--client` flags.
    // CLI flags always win — predictable semantics over "merge". Unknown /
    // typo'd ids are dropped silently so a stale settings.json entry never
    // breaks tokscale.
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

fn client_filter_explicitly_requests_warp(clients: &Option<Vec<String>>) -> bool {
    clients
        .as_ref()
        .is_some_and(|sources| sources.iter().any(|source| source == "warp"))
}

#[derive(Debug)]
struct CursorSetupState {
    has_credentials: bool,
    has_cache: bool,
    cache_glob: String,
    home_override: bool,
}

fn cursor_setup_state(home_dir: &Option<String>) -> Option<CursorSetupState> {
    let (home_path, home_override) = match home_dir {
        Some(home) => (PathBuf::from(home), true),
        None => (dirs::home_dir()?, false),
    };
    let has_credentials = if home_override {
        cursor::has_active_credentials_in_home(&home_path)
    } else {
        cursor::is_cursor_logged_in()
    };
    let has_cache = cursor::has_cursor_usage_cache_in_home(&home_path);
    let cache_glob = if home_override {
        home_path
            .join(".config/tokscale/cursor-cache/usage*.csv")
            .to_string_lossy()
            .to_string()
    } else {
        "~/.config/tokscale/cursor-cache/usage*.csv".to_string()
    };

    Some(CursorSetupState {
        has_credentials,
        has_cache,
        cache_glob,
        home_override,
    })
}

fn has_cursor_usage_cache_for_report(home_dir: &Option<String>) -> bool {
    cursor_setup_state(home_dir).is_some_and(|state| state.has_cache)
}

fn cursor_setup_warnings_for_report(
    home_dir: &Option<String>,
    clients: &Option<Vec<String>>,
) -> Vec<String> {
    if !client_filter_explicitly_requests_cursor(clients) {
        return Vec::new();
    }

    let Some(state) = cursor_setup_state(home_dir) else {
        return vec![
            "Cursor usage requires Tokscale's Cursor API cache, but the home directory could not be resolved. Run `tokscale cursor login` and `tokscale cursor sync --json`. Tokscale does not parse local `~/.cursor` session data.".to_string(),
        ];
    };
    if state.has_cache {
        return Vec::new();
    }

    let action = if state.home_override {
        "run `tokscale cursor login` and `tokscale cursor sync --json`, or populate that cache before running a report with --home"
    } else if state.has_credentials {
        "run `tokscale cursor sync --json`"
    } else {
        "run `tokscale cursor login` and `tokscale cursor sync --json`"
    };

    vec![format!(
        "Cursor usage requires Tokscale's Cursor API cache at `{}`; {}. Tokscale does not parse local `~/.cursor` session data.",
        state.cache_glob, action
    )]
}

fn emit_cursor_setup_warnings(warnings: &[String]) {
    if warnings.is_empty() {
        return;
    }

    use colored::Colorize;
    for warning in warnings {
        eprintln!("{}", format!("  Warning: {}", warning).yellow());
    }
}

fn warp_setup_warnings_for_report(
    home_dir: &Option<String>,
    clients: &Option<Vec<String>>,
) -> Vec<String> {
    if !client_filter_explicitly_requests_warp(clients) {
        return Vec::new();
    }

    let (home_path, home_override) = match home_dir {
        Some(home) => (PathBuf::from(home), true),
        None => match dirs::home_dir() {
            Some(home) => (home, false),
            None => {
                return vec![
                    "Warp usage requires Tokscale's Warp aggregate cache, but the home directory could not be resolved. Tokscale does not parse local Warp transcripts.".to_string(),
                ];
            }
        },
    };
    let has_cache = if home_override {
        warp::has_usage_cache_in_home(&home_path)
    } else {
        warp::load_usage_cache().is_some()
    };
    if has_cache {
        return Vec::new();
    }

    let cache_glob = if home_override {
        home_path
            .join(".config/tokscale/warp-cache/usage*.json")
            .to_string_lossy()
            .to_string()
    } else {
        "~/.config/tokscale/warp-cache/usage*.json".to_string()
    };
    let action = if home_override {
        "run `tokscale warp sync` for the default profile or populate that cache before running a report with --home"
    } else if warp::has_credentials() {
        "run `tokscale warp sync`"
    } else {
        "run `tokscale warp login` and `tokscale warp sync`"
    };

    vec![format!(
        "Warp usage requires Tokscale's aggregate API cache at `{}`; {}. Tokscale does not parse local Warp/Oz session transcripts and does not infer tokens from request counts.",
        cache_glob, action
    )]
}

fn setup_warnings_for_report(
    home_dir: &Option<String>,
    clients: &Option<Vec<String>>,
) -> Vec<String> {
    let mut warnings = cursor_setup_warnings_for_report(home_dir, clients);
    warnings.extend(warp_setup_warnings_for_report(home_dir, clients));
    warnings
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

    // Skip the implicit refresh when each expected Cursor account cache is
    // recent enough — running `tokscale models` 30× in a script must not
    // produce 30 Cursor API calls. The manual `tokscale cursor sync` command
    // bypasses this gate.
    if cursor::cursor_usage_cache_is_fresh(cursor::CURSOR_AUTO_SYNC_FRESHNESS) {
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
    let had_cursor_cache = has_cursor_usage_cache_for_report(home_dir);
    let explicit_cursor_filter = client_filter_explicitly_requests_cursor(clients);
    let cursor_sync_result = auto_sync_cursor_for_local_report(home_dir, clients);
    emit_cursor_sync_warning(
        cursor_sync_result.as_ref(),
        had_cursor_cache,
        explicit_cursor_filter,
    );
    let cursor_setup_warnings = setup_warnings_for_report(home_dir, clients);
    emit_cursor_setup_warnings(&cursor_setup_warnings);
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

fn resolve_effective_home_dir(home_dir: &Option<String>) -> Option<PathBuf> {
    home_dir.as_ref().map(PathBuf::from).or_else(dirs::home_dir)
}

fn model_usage_includes_client(entry: &tokscale_core::ModelUsage, client: &str) -> bool {
    if entry.client == client {
        return true;
    }

    entry
        .merged_clients
        .as_deref()
        .is_some_and(|clients| clients.split(", ").any(|id| id == client))
}

fn emit_client_diagnostics(diagnostics: &[claude_diagnostics::ClientDiagnostic]) {
    if diagnostics.is_empty() {
        return;
    }

    use colored::Colorize;
    for diagnostic in diagnostics {
        eprintln!(
            "{}",
            format!("  {}: {}", diagnostic.severity, diagnostic.message).yellow()
        );
        eprintln!("{}", format!("  {}", diagnostic.help).bright_black());
    }
}

fn ensure_home_supported_for_tui(home_dir: &Option<String>) -> Result<()> {
    if home_dir.is_some() {
        return Err(anyhow::anyhow!(
            "--home is currently supported for local report commands only. Use `--json`, `--light`, `models`, `monthly`, or `graph` instead of TUI mode."
        ));
    }

    Ok(())
}

fn build_date_filter(date: &DateRangeFlags) -> (Option<String>, Option<String>) {
    build_date_filter_for_date(date, chrono::Local::now().date_naive())
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

fn normalize_year_filter(date: &DateRangeFlags) -> Option<String> {
    if date.today || date.yesterday || date.week || date.month {
        None
    } else {
        date.year.clone()
    }
}

fn get_date_range_label(date: &DateRangeFlags) -> Option<String> {
    get_date_range_label_for_date(date, chrono::Local::now().date_naive())
}

fn get_date_range_label_for_date(
    date: &DateRangeFlags,
    current_date: chrono::NaiveDate,
) -> Option<String> {
    if date.today {
        return Some("Today".to_string());
    }
    if date.yesterday {
        return Some("Yesterday".to_string());
    }
    if date.week {
        return Some("Last 7 days".to_string());
    }
    if date.month {
        return Some(current_date.format("%B %Y").to_string());
    }
    if let Some(y) = &date.year {
        return Some(y.clone());
    }
    let mut parts = Vec::new();
    if let Some(s) = &date.since {
        parts.push(format!("from {}", s));
    }
    if let Some(u) = &date.until {
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
    date: &DateRangeFlags,
    benchmark: bool,
    no_spinner: bool,
    group_by: tokscale_core::GroupBy,
    cli_write_cache: bool,
    cli_no_write_cache: bool,
) -> Result<()> {
    use std::time::Instant;
    use tokio::runtime::Runtime;
    use tokscale_core::{get_model_report, GroupBy, ReportOptions};

    let (since, until) = build_date_filter(date);
    let year = normalize_year_filter(date);
    let date_range = get_date_range_label(date);
    let effective_home_dir = resolve_effective_home_dir(&home_dir);

    let had_cursor_cache = has_cursor_usage_cache_for_report(&home_dir);
    let explicit_cursor_filter = client_filter_explicitly_requests_cursor(&clients);
    let spinner = if no_spinner {
        None
    } else {
        Some(LightSpinner::start("Scanning session data..."))
    };
    let cursor_sync_result = auto_sync_cursor_for_local_report(&home_dir, &clients);
    let cursor_setup_warnings = setup_warnings_for_report(&home_dir, &clients);
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
                scanner_settings: tui::settings::load_scanner_settings_for_home(&home_dir),
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
    let claude_message_count = report
        .entries
        .iter()
        .filter(|entry| model_usage_includes_client(entry, "claude"))
        .map(|entry| entry.message_count)
        .sum();
    let diagnostics = effective_home_dir
        .as_deref()
        .map(|home| {
            claude_diagnostics::diagnostics_for_empty_explicit_report(
                home,
                &clients,
                claude_message_count,
            )
        })
        .unwrap_or_default();

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
            #[serde(skip_serializing_if = "Option::is_none")]
            session_id: Option<String>,
            model: String,
            provider: String,
            input: i64,
            output: i64,
            cache_read: i64,
            cache_write: i64,
            reasoning: i64,
            message_count: i32,
            cost: f64,
            performance: tokscale_core::ModelPerformance,
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
            #[serde(skip_serializing_if = "Vec::is_empty")]
            warnings: Vec<String>,
            #[serde(skip_serializing_if = "Vec::is_empty")]
            diagnostics: Vec<claude_diagnostics::ClientDiagnostic>,
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
                    session_id: if matches!(group_by, GroupBy::Session | GroupBy::ClientSession) {
                        e.session_id
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
                    performance: e.performance,
                })
                .collect(),
            total_input: report.total_input,
            total_output: report.total_output,
            total_cache_read: report.total_cache_read,
            total_cache_write: report.total_cache_write,
            total_messages: report.total_messages,
            total_cost: report.total_cost,
            processing_time_ms: report.processing_time_ms,
            warnings: cursor_setup_warnings,
            diagnostics,
        };
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        use comfy_table::{Attribute, Cell, CellAlignment, Color, ContentArrangement, Table};
        emit_client_diagnostics(&diagnostics);

        emit_cursor_setup_warnings(&cursor_setup_warnings);
        let total_performance = aggregate_model_report_performance(&report.entries);
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
                        Cell::new("ms/1K").fg(Color::Cyan),
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
                            Cell::new(crate::tui::ui::widgets::get_provider_display_name(
                                &entry.provider,
                            ))
                            .add_attribute(Attribute::Dim),
                            Cell::new(&entry.model),
                            Cell::new(format_tokens_with_commas(entry.input))
                                .set_alignment(CellAlignment::Right),
                            Cell::new(format_tokens_with_commas(entry.output))
                                .set_alignment(CellAlignment::Right),
                            Cell::new(format_ms_per_1k(entry.performance.ms_per_1k_tokens))
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
                        Cell::new(format_ms_per_1k(total_performance.ms_per_1k_tokens))
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
                        Cell::new("ms/1K").fg(Color::Cyan),
                        Cell::new("Cost").fg(Color::Cyan),
                        Cell::new("Cost/1M").fg(Color::Cyan),
                    ]);

                    for entry in &report.entries {
                        let total_tokens =
                            entry.input + entry.output + entry.cache_read + entry.cache_write;
                        table.add_row(vec![
                            Cell::new(capitalize_client(&entry.client)),
                            Cell::new(crate::tui::ui::widgets::get_provider_display_name(
                                &entry.provider,
                            ))
                            .add_attribute(Attribute::Dim),
                            Cell::new(&entry.model),
                            Cell::new(format_tokens_with_commas(entry.input))
                                .set_alignment(CellAlignment::Right),
                            Cell::new(format_tokens_with_commas(entry.output))
                                .set_alignment(CellAlignment::Right),
                            Cell::new(format_ms_per_1k(entry.performance.ms_per_1k_tokens))
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
                        Cell::new(format_ms_per_1k(total_performance.ms_per_1k_tokens))
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
                GroupBy::Session | GroupBy::ClientSession => {
                    let show_client = group_by == GroupBy::ClientSession;
                    let mut header = Vec::with_capacity(6);
                    if show_client {
                        header.push(Cell::new("Client").fg(Color::Cyan));
                    }
                    header.extend([
                        Cell::new("Session").fg(Color::Cyan),
                        Cell::new("Model").fg(Color::Cyan),
                        Cell::new("Total").fg(Color::Cyan),
                        Cell::new("Cost").fg(Color::Cyan),
                    ]);
                    table.set_header(header);

                    for entry in &report.entries {
                        let total_tokens =
                            entry.input + entry.output + entry.cache_read + entry.cache_write;
                        let session_label = entry
                            .session_id
                            .clone()
                            .unwrap_or_else(|| "(unknown)".to_string());
                        let mut row = Vec::with_capacity(6);
                        if show_client {
                            row.push(Cell::new(capitalize_client(&entry.client)));
                        }
                        row.extend([
                            Cell::new(session_label),
                            Cell::new(&entry.model),
                            Cell::new(format_tokens_with_commas(total_tokens))
                                .set_alignment(CellAlignment::Right),
                            Cell::new(format_currency(entry.cost))
                                .set_alignment(CellAlignment::Right),
                        ]);
                        table.add_row(row);
                    }

                    let total_all = report.total_input
                        + report.total_output
                        + report.total_cache_read
                        + report.total_cache_write;
                    let mut total_row = Vec::with_capacity(6);
                    if show_client {
                        total_row.push(
                            Cell::new("Total")
                                .fg(Color::Yellow)
                                .add_attribute(Attribute::Bold),
                        );
                        total_row.push(Cell::new(""));
                    } else {
                        total_row.push(
                            Cell::new("Total")
                                .fg(Color::Yellow)
                                .add_attribute(Attribute::Bold),
                        );
                    }
                    total_row.push(Cell::new(""));
                    total_row.push(
                        Cell::new(format_tokens_with_commas(total_all))
                            .fg(Color::Yellow)
                            .set_alignment(CellAlignment::Right),
                    );
                    total_row.push(
                        Cell::new(format_currency(report.total_cost))
                            .fg(Color::Yellow)
                            .set_alignment(CellAlignment::Right),
                    );
                    table.add_row(total_row);
                }
                GroupBy::WorkspaceModel => {
                    table.set_header(vec![
                        Cell::new("Workspace").fg(Color::Cyan),
                        Cell::new("Model").fg(Color::Cyan),
                        Cell::new("ms/1K").fg(Color::Cyan),
                        Cell::new("Cost").fg(Color::Cyan),
                    ]);

                    for entry in &report.entries {
                        table.add_row(vec![
                            Cell::new(workspace_name(entry.workspace_label.as_deref())),
                            Cell::new(&entry.model),
                            Cell::new(format_ms_per_1k(entry.performance.ms_per_1k_tokens))
                                .set_alignment(CellAlignment::Right),
                            Cell::new(format_currency(entry.cost))
                                .set_alignment(CellAlignment::Right),
                        ]);
                    }

                    table.add_row(vec![
                        Cell::new("Total")
                            .fg(Color::Yellow)
                            .add_attribute(Attribute::Bold),
                        Cell::new(""),
                        Cell::new(format_ms_per_1k(total_performance.ms_per_1k_tokens))
                            .fg(Color::Yellow)
                            .set_alignment(CellAlignment::Right),
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
                        Cell::new("ms/1K").fg(Color::Cyan),
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
                            Cell::new(crate::tui::ui::widgets::get_provider_display_name(
                                &entry.provider,
                            ))
                            .add_attribute(Attribute::Dim),
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
                            Cell::new(format_ms_per_1k(entry.performance.ms_per_1k_tokens))
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
                        Cell::new(format_ms_per_1k(total_performance.ms_per_1k_tokens))
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
                GroupBy::Session | GroupBy::ClientSession => {
                    let show_client = group_by == GroupBy::ClientSession;
                    let mut header = Vec::with_capacity(9);
                    if show_client {
                        header.push(Cell::new("Client").fg(Color::Cyan));
                    }
                    header.extend([
                        Cell::new("Session").fg(Color::Cyan),
                        Cell::new("Provider").fg(Color::Cyan),
                        Cell::new("Model").fg(Color::Cyan),
                        Cell::new("Input").fg(Color::Cyan),
                        Cell::new("Output").fg(Color::Cyan),
                        Cell::new("Total").fg(Color::Cyan),
                        Cell::new("Cost").fg(Color::Cyan),
                        Cell::new("Cost/1M").fg(Color::Cyan),
                    ]);
                    table.set_header(header);

                    for entry in &report.entries {
                        let total =
                            entry.input + entry.output + entry.cache_write + entry.cache_read;
                        let session_label = entry
                            .session_id
                            .clone()
                            .unwrap_or_else(|| "(unknown)".to_string());
                        let mut row = Vec::with_capacity(9);
                        if show_client {
                            row.push(Cell::new(capitalize_client(&entry.client)));
                        }
                        row.extend([
                            Cell::new(session_label),
                            Cell::new(crate::tui::ui::widgets::get_provider_display_name(
                                &entry.provider,
                            ))
                            .add_attribute(Attribute::Dim),
                            Cell::new(&entry.model),
                            Cell::new(format_tokens_with_commas(entry.input))
                                .set_alignment(CellAlignment::Right),
                            Cell::new(format_tokens_with_commas(entry.output))
                                .set_alignment(CellAlignment::Right),
                            Cell::new(format_tokens_with_commas(total))
                                .set_alignment(CellAlignment::Right),
                            Cell::new(format_currency(entry.cost))
                                .set_alignment(CellAlignment::Right),
                            Cell::new(format_cost_per_million(entry.cost, total))
                                .set_alignment(CellAlignment::Right),
                        ]);
                        table.add_row(row);
                    }

                    let total_all = report.total_input
                        + report.total_output
                        + report.total_cache_write
                        + report.total_cache_read;
                    let mut total_row: Vec<Cell> = Vec::with_capacity(9);
                    total_row.push(
                        Cell::new("Total")
                            .fg(Color::Yellow)
                            .add_attribute(Attribute::Bold),
                    );
                    let blanks = if show_client { 3 } else { 2 };
                    for _ in 0..blanks {
                        total_row.push(Cell::new(""));
                    }
                    total_row.push(
                        Cell::new(format_tokens_with_commas(report.total_input))
                            .fg(Color::Yellow)
                            .set_alignment(CellAlignment::Right),
                    );
                    total_row.push(
                        Cell::new(format_tokens_with_commas(report.total_output))
                            .fg(Color::Yellow)
                            .set_alignment(CellAlignment::Right),
                    );
                    total_row.push(
                        Cell::new(format_tokens_with_commas(total_all))
                            .fg(Color::Yellow)
                            .set_alignment(CellAlignment::Right),
                    );
                    total_row.push(
                        Cell::new(format_currency(report.total_cost))
                            .fg(Color::Yellow)
                            .set_alignment(CellAlignment::Right),
                    );
                    total_row.push(
                        Cell::new(format_cost_per_million(report.total_cost, total_all))
                            .fg(Color::Yellow)
                            .set_alignment(CellAlignment::Right),
                    );
                    table.add_row(total_row);
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
                        Cell::new("ms/1K").fg(Color::Cyan),
                        Cell::new("Cost").fg(Color::Cyan),
                        Cell::new("Cost/1M").fg(Color::Cyan),
                    ]);

                    for entry in &report.entries {
                        let total =
                            entry.input + entry.output + entry.cache_write + entry.cache_read;

                        table.add_row(vec![
                            Cell::new(capitalize_client(&entry.client)),
                            Cell::new(crate::tui::ui::widgets::get_provider_display_name(
                                &entry.provider,
                            ))
                            .add_attribute(Attribute::Dim),
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
                            Cell::new(format_ms_per_1k(entry.performance.ms_per_1k_tokens))
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
                        Cell::new(format_ms_per_1k(total_performance.ms_per_1k_tokens))
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
                        Cell::new("ms/1K").fg(Color::Cyan),
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
                            Cell::new(crate::tui::ui::widgets::get_provider_display_name(
                                &entry.provider,
                            ))
                            .add_attribute(Attribute::Dim),
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
                            Cell::new(format_ms_per_1k(entry.performance.ms_per_1k_tokens))
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
                        Cell::new(format_ms_per_1k(total_performance.ms_per_1k_tokens))
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

fn run_monthly_report(
    json: bool,
    home_dir: Option<String>,
    clients: Option<Vec<String>>,
    date: &DateRangeFlags,
    benchmark: bool,
    no_spinner: bool,
) -> Result<()> {
    use std::time::Instant;
    use tokio::runtime::Runtime;
    use tokscale_core::{get_monthly_report, GroupBy, ReportOptions};

    let (since, until) = build_date_filter(date);
    let year = normalize_year_filter(date);
    let date_range = get_date_range_label(date);

    let had_cursor_cache = has_cursor_usage_cache_for_report(&home_dir);
    let explicit_cursor_filter = client_filter_explicitly_requests_cursor(&clients);
    let spinner = if no_spinner {
        None
    } else {
        Some(LightSpinner::start("Scanning session data..."))
    };
    let cursor_sync_result = auto_sync_cursor_for_local_report(&home_dir, &clients);
    let cursor_setup_warnings = setup_warnings_for_report(&home_dir, &clients);
    let use_env_roots = use_env_roots(&home_dir);
    let start = Instant::now();
    let rt = Runtime::new()?;
    let report = rt
        .block_on(async {
            get_monthly_report(ReportOptions {
                home_dir: home_dir.clone(),
                use_env_roots,
                clients,
                since,
                until,
                year,
                group_by: GroupBy::default(),
                scanner_settings: tui::settings::load_scanner_settings_for_home(&home_dir),
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
            #[serde(skip_serializing_if = "Vec::is_empty")]
            warnings: Vec<String>,
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
            warnings: cursor_setup_warnings,
        };

        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        use comfy_table::{Attribute, Cell, CellAlignment, Color, ContentArrangement, Table};

        emit_cursor_setup_warnings(&cursor_setup_warnings);
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

fn run_hourly_report(
    json: bool,
    home_dir: Option<String>,
    clients: Option<Vec<String>>,
    date: &DateRangeFlags,
    benchmark: bool,
    no_spinner: bool,
) -> Result<()> {
    use std::time::Instant;
    use tokio::runtime::Runtime;
    use tokscale_core::{get_hourly_report, GroupBy, ReportOptions};

    let (since, until) = build_date_filter(date);
    let year = normalize_year_filter(date);
    let date_range = get_date_range_label(date);

    let had_cursor_cache = has_cursor_usage_cache_for_report(&home_dir);
    let explicit_cursor_filter = client_filter_explicitly_requests_cursor(&clients);
    let spinner = if no_spinner {
        None
    } else {
        Some(LightSpinner::start("Scanning session data..."))
    };
    let cursor_sync_result = auto_sync_cursor_for_local_report(&home_dir, &clients);
    let cursor_setup_warnings = setup_warnings_for_report(&home_dir, &clients);
    let use_env_roots = use_env_roots(&home_dir);
    let start = Instant::now();
    let rt = Runtime::new()?;
    let report = rt
        .block_on(async {
            get_hourly_report(ReportOptions {
                home_dir: home_dir.clone(),
                use_env_roots,
                clients,
                since,
                until,
                year,
                group_by: GroupBy::default(),
                scanner_settings: tui::settings::load_scanner_settings_for_home(&home_dir),
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
            #[serde(skip_serializing_if = "Vec::is_empty")]
            warnings: Vec<String>,
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
            warnings: cursor_setup_warnings,
        };

        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        use comfy_table::{Cell, CellAlignment, Color, ContentArrangement, Table};

        emit_cursor_setup_warnings(&cursor_setup_warnings);
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
    show_clients: bool,
    disable_pinned: bool,
) -> Result<()> {
    use colored::Colorize;

    println!("{}", "\n  Tokscale - Generate Wrapped Image\n".cyan());

    println!("{}", "  Generating wrapped image...".bright_black());
    println!();

    let include_agents = !show_clients || agents;
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

    if model_id.eq_ignore_ascii_case("list-overrides") {
        return run_pricing_list_overrides(json);
    }

    let provider_normalized = provider.map(|p| p.to_lowercase());
    if let Some(ref p) = provider_normalized {
        if p != "custom" && p != "litellm" && p != "openrouter" && p != "models.dev" {
            println!(
                "\n  {}",
                format!("Invalid provider: {}", provider.unwrap_or("")).red()
            );
            println!(
                "{}\n",
                "  Valid providers: custom, litellm, openrouter, models.dev".bright_black()
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
                let source_label = match pricing.source.to_lowercase().as_str() {
                    "custom" => "Custom",
                    "litellm" => "LiteLLM",
                    "openrouter" => "OpenRouter",
                    "models.dev" => "Models.dev",
                    _ => pricing.source.as_str(),
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

fn run_pricing_list_overrides(json: bool) -> Result<()> {
    use colored::Colorize;
    use tokscale_core::pricing::custom::CustomPricing;
    use tokscale_core::pricing::ModelPricing;

    fn per_million(value: Option<f64>) -> Option<f64> {
        value.map(|v| v * 1_000_000.0)
    }

    #[derive(serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    struct OverrideEntry {
        model_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        input_cost_per_million_tokens: Option<f64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        output_cost_per_million_tokens: Option<f64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_read_input_token_cost_per_million_tokens: Option<f64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_creation_input_token_cost_per_million_tokens: Option<f64>,
    }

    fn entry(model_id: &str, pricing: &ModelPricing) -> OverrideEntry {
        OverrideEntry {
            model_id: model_id.to_string(),
            input_cost_per_million_tokens: per_million(pricing.input_cost_per_token),
            output_cost_per_million_tokens: per_million(pricing.output_cost_per_token),
            cache_read_input_token_cost_per_million_tokens: per_million(
                pricing.cache_read_input_token_cost,
            ),
            cache_creation_input_token_cost_per_million_tokens: per_million(
                pricing.cache_creation_input_token_cost,
            ),
        }
    }

    let path = CustomPricing::default_path();
    let overrides = CustomPricing::load_from_path(&path);
    let mut entries: Vec<OverrideEntry> = overrides
        .entries()
        .map(|(model_id, pricing)| entry(model_id, pricing))
        .collect();
    entries.sort_by(|a, b| a.model_id.cmp(&b.model_id));

    if json {
        #[derive(serde::Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Output {
            path: String,
            count: usize,
            models: Vec<OverrideEntry>,
        }

        println!(
            "{}",
            serde_json::to_string_pretty(&Output {
                path: path.display().to_string(),
                count: entries.len(),
                models: entries,
            })?
        );
        return Ok(());
    }

    if entries.is_empty() {
        println!(
            "\n  {}\n  Tried: {}\n",
            "No custom pricing overrides loaded".yellow(),
            path.display()
        );
        return Ok(());
    }

    println!("\n  {}", "Custom pricing overrides".bold());
    println!("  Path: {}", path.display());
    println!("  Loaded once at startup; restart tokscale after editing this file.");
    println!();

    for entry in entries {
        println!("  {}", entry.model_id.bold());
        if let Some(input) = entry.input_cost_per_million_tokens {
            println!("    Input:  ${:.2} / 1M tokens", input);
        }
        if let Some(output) = entry.output_cost_per_million_tokens {
            println!("    Output: ${:.2} / 1M tokens", output);
        }
        if let Some(cache_read) = entry.cache_read_input_token_cost_per_million_tokens {
            println!("    Cache Read:  ${:.2} / 1M tokens", cache_read);
        }
        if let Some(cache_write) = entry.cache_creation_input_token_cost_per_million_tokens {
            println!("    Cache Write: ${:.2} / 1M tokens", cache_write);
        }
    }
    println!();

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

fn format_ms_per_1k(ms_per_1k_tokens: Option<f64>) -> String {
    let Some(value) = ms_per_1k_tokens else {
        return "—".to_string();
    };
    if !value.is_finite() || value <= 0.0 {
        "—".to_string()
    } else if value >= 1000.0 {
        format!("{:.1}s", value / 1000.0)
    } else {
        format!("{:.0}ms", value)
    }
}

fn model_entry_total_tokens(entry: &tokscale_core::ModelUsage) -> i64 {
    entry.input.max(0)
        + entry.output.max(0)
        + entry.cache_read.max(0)
        + entry.cache_write.max(0)
        + entry.reasoning.max(0)
}

fn aggregate_model_report_performance(
    entries: &[tokscale_core::ModelUsage],
) -> tokscale_core::ModelPerformance {
    let mut performance = tokscale_core::ModelPerformance::default();
    for entry in entries {
        performance.total_duration_ms = performance
            .total_duration_ms
            .saturating_add(entry.performance.total_duration_ms);
        performance.timed_tokens = performance
            .timed_tokens
            .saturating_add(entry.performance.timed_tokens);
        performance.sample_count = performance
            .sample_count
            .saturating_add(entry.performance.sample_count);
    }
    let total_tokens = entries.iter().map(model_entry_total_tokens).sum();
    performance.finalize(total_tokens);
    performance
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
        "warp" => "Warp".to_string(),
        "grok" => "Grok Build".to_string(),
        "pi" => "Pi".to_string(),
        "gjc" => "Gajae-Code".to_string(),
        "jcode" => "Jcode".to_string(),
        "commandcode" => "Command Code".to_string(),
        "junie" => "Junie".to_string(),
        "zcode" => "ZCode".to_string(),
        other => other.to_string(),
    }
}

fn run_clients_command(json: bool, home_dir: Option<String>) -> Result<()> {
    use tokscale_core::{
        built_in_extra_scan_paths_for, extra_scan_paths_for, parse_local_clients, ClientId,
        LocalParseOptions,
    };

    let explicit_home_dir = home_dir;
    let use_env_roots = use_env_roots(&explicit_home_dir);
    let scanner_settings = tui::settings::load_scanner_settings_for_home(&explicit_home_dir);
    let home_dir = explicit_home_dir
        .map(PathBuf::from)
        .or_else(dirs::home_dir)
        .ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?;
    let home_dir_str = home_dir.to_string_lossy().to_string();

    let parsed = parse_local_clients(LocalParseOptions {
        home_dir: Some(home_dir_str.clone()),
        use_env_roots,
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

    let headless_roots =
        tokscale_core::scanner::headless_roots_with_env_strategy(&home_dir_str, use_env_roots);
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
        additional_paths: Vec<AdditionalPath>,
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
        #[serde(skip_serializing_if = "Vec::is_empty")]
        diagnostics: Vec<claude_diagnostics::ClientDiagnostic>,
    }

    #[derive(serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    struct AdditionalPath {
        path: String,
        exists: bool,
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

    let all_clients: std::collections::HashSet<ClientId> = ClientId::iter().collect();
    let extra_dirs: Vec<(ClientId, String)> = if use_env_roots {
        let extra_dirs_val = std::env::var("TOKSCALE_EXTRA_DIRS").unwrap_or_default();
        tokscale_core::parse_extra_dirs(&extra_dirs_val, &all_clients)
    } else {
        Vec::new()
    };
    let built_in_extra_paths = built_in_extra_scan_paths_for(&home_dir_str, &all_clients);
    let settings_extra_dirs = extra_scan_paths_for(&scanner_settings, &all_clients);
    let copilot_exporter_path =
        tokscale_core::copilot_exporter_path_with_env_strategy(use_env_roots);

    let clients: Vec<ClientRow> =
        ClientId::iter()
            .map(|client| {
                let sessions_path = client
                    .data()
                    .resolve_path_with_env_strategy(&home_dir_str, use_env_roots);
                let sessions_path_exists = Path::new(&sessions_path).exists();
                let additional_paths: Vec<AdditionalPath> = built_in_extra_paths
                    .iter()
                    .filter(|(c, _)| *c == client)
                    .map(|(_, path)| AdditionalPath {
                        path: path.to_string_lossy().to_string(),
                        exists: path.exists(),
                    })
                    .collect();
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
                    ClientId::AntigravityCli => "Antigravity CLI",
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

                let diagnostics = if client == ClientId::Claude {
                    claude_diagnostics::diagnostics_for_clients_row(&home_dir)
                } else {
                    Vec::new()
                };

                ClientRow {
                    client: client.as_str().to_string(),
                    label,
                    sessions_path,
                    sessions_path_exists,
                    additional_paths,
                    legacy_paths,
                    message_count: parsed.counts.get(client),
                    headless_supported,
                    headless_paths,
                    headless_message_count,
                    exporter_status: (client == ClientId::Copilot
                        && copilot_exporter_path.is_some())
                    .then(|| "configured".to_string()),
                    extra_paths,
                    diagnostics,
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
                    describe_path_for_home(&row.sessions_path, row.sessions_path_exists, &home_dir)
                )
                .bright_black()
            );

            if !row.additional_paths.is_empty() {
                let additional_desc: Vec<String> = row
                    .additional_paths
                    .iter()
                    .map(|ap| describe_path_for_home(&ap.path, ap.exists, &home_dir))
                    .collect();
                println!(
                    "  {}",
                    format!("additional: {}", additional_desc.join(", ")).bright_black()
                );
            }

            if !row.legacy_paths.is_empty() {
                let legacy_desc: Vec<String> = row
                    .legacy_paths
                    .iter()
                    .map(|lp| describe_path_for_home(&lp.path, lp.exists, &home_dir))
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
                    .map(|ep| describe_path_for_home(&ep.path, ep.exists, &home_dir))
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
                    .map(|ep| describe_path_for_home(&ep.path, ep.exists, &home_dir))
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
                    .map(|hp| describe_path_for_home(&hp.path, hp.exists, &home_dir))
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

            for diagnostic in &row.diagnostics {
                println!(
                    "  {}",
                    format!("{}: {}", diagnostic.severity, diagnostic.message).yellow()
                );
                println!("  {}", diagnostic.help.bright_black());
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

fn describe_path_for_home(path: &str, exists: bool, home: &Path) -> String {
    let path_display = path.replace(&home.to_string_lossy().to_string(), "~");
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
    #[serde(skip_serializing_if = "Option::is_none")]
    active_time_ms: Option<i64>,
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
struct TsSubmitDevice {
    id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct TsTimeMetrics {
    total_active_time_ms: i64,
    longest_continuous_ms: i64,
    max_concurrent_sessions: u32,
    session_count: u32,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct TsTokenContributionData {
    meta: TsExportMeta,
    #[serde(skip_serializing_if = "Option::is_none")]
    device: Option<TsSubmitDevice>,
    summary: TsDataSummary,
    years: Vec<TsYearSummary>,
    contributions: Vec<TsDailyContribution>,
    #[serde(skip_serializing_if = "Option::is_none")]
    time_metrics: Option<TsTimeMetrics>,
    #[serde(skip_serializing_if = "Option::is_none")]
    mcp_servers: Option<Vec<String>>,
}

fn to_ts_token_contribution_data(
    graph: &tokscale_core::GraphResult,
    device: Option<&device::SubmitDevice>,
) -> TsTokenContributionData {
    TsTokenContributionData {
        meta: TsExportMeta {
            generated_at: graph.meta.generated_at.clone(),
            version: graph.meta.version.clone(),
            date_range: DateRange {
                start: graph.meta.date_range_start.clone(),
                end: graph.meta.date_range_end.clone(),
            },
        },
        device: device.map(|d| TsSubmitDevice {
            id: d.id.clone(),
            name: d.name.clone(),
        }),
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
                active_time_ms: d.active_time_ms,
            })
            .collect(),
        time_metrics: graph.time_metrics.as_ref().map(|tm| TsTimeMetrics {
            total_active_time_ms: tm.total_active_time_ms,
            longest_continuous_ms: tm.longest_continuous_ms,
            max_concurrent_sessions: tm.max_concurrent_sessions,
            session_count: tm.session_count,
        }),
        mcp_servers: {
            let servers = tokscale_core::mcp::discover_mcp_server_names(None);
            if servers.is_empty() {
                None
            } else {
                Some(servers)
            }
        },
    }
}

fn run_login_command(token: Option<String>) -> Result<()> {
    use tokio::runtime::Runtime;

    let rt = Runtime::new()?;
    rt.block_on(async {
        match token {
            Some(token) => auth::login_with_token(&token).await,
            None => auth::login().await,
        }
    })
}

fn run_logout_command() -> Result<()> {
    auth::logout()
}

fn run_whoami_command() -> Result<()> {
    auth::whoami()
}

fn run_qr_command(yes: bool) -> Result<()> {
    auth::show_qr(yes)
}

fn run_delete_data_command() -> Result<()> {
    use colored::Colorize;
    use std::io::{self, Write};
    use tokio::runtime::Runtime;

    let auth_token = auth::resolve_api_token().ok_or_else(|| {
        anyhow::anyhow!("Not logged in. Run `tokscale login` or set TOKSCALE_API_TOKEN.")
    })?;

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
            .header("Authorization", format!("Bearer {}", auth_token.token))
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
fn run_time_metrics_report(
    json: bool,
    home_dir: Option<String>,
    clients: Option<Vec<String>>,
    since: Option<String>,
    until: Option<String>,
    year: Option<String>,
    no_spinner: bool,
) -> Result<()> {
    use tokio::runtime::Runtime;
    use tokscale_core::{get_time_metrics_report, GroupBy, ReportOptions};

    let had_cursor_cache = has_cursor_usage_cache_for_report(&home_dir);
    let explicit_cursor_filter = client_filter_explicitly_requests_cursor(&clients);
    let spinner = if no_spinner {
        None
    } else {
        Some(LightSpinner::start("Computing time metrics..."))
    };
    let cursor_sync_result = auto_sync_cursor_for_local_report(&home_dir, &clients);
    let cursor_setup_warnings = setup_warnings_for_report(&home_dir, &clients);
    let use_env_roots = use_env_roots(&home_dir);
    let rt = Runtime::new()?;
    let report = rt
        .block_on(async {
            get_time_metrics_report(ReportOptions {
                home_dir: home_dir.clone(),
                use_env_roots,
                clients,
                since,
                until,
                year,
                group_by: GroupBy::default(),
                scanner_settings: tui::settings::load_scanner_settings_for_home(&home_dir),
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

    let m = &report.metrics;

    if json {
        #[derive(serde::Serialize)]
        #[serde(rename_all = "camelCase")]
        struct TimeMetricsReportJson<'a> {
            metrics: &'a tokscale_core::TimeMetrics,
            processing_time_ms: u32,
            #[serde(skip_serializing_if = "Vec::is_empty")]
            warnings: Vec<String>,
        }

        let output = TimeMetricsReportJson {
            metrics: &report.metrics,
            processing_time_ms: report.processing_time_ms,
            warnings: cursor_setup_warnings,
        };
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        emit_cursor_setup_warnings(&cursor_setup_warnings);
        println!("Session Time Metrics");
        println!("====================");
        println!(
            "Total active time:       {}",
            format_duration_ms(m.total_active_time_ms)
        );
        println!(
            "Total wall-clock time:   {}",
            format_duration_ms(m.total_wall_time_ms)
        );
        println!(
            "Longest continuous use:  {}",
            format_duration_ms(m.longest_continuous_ms)
        );
        println!("Max concurrent sessions: {}", m.max_concurrent_sessions);
        println!("Total sessions:          {}", m.session_count);
        println!("Processing time:         {}ms", report.processing_time_ms);
    }

    Ok(())
}

fn format_duration_ms(ms: i64) -> String {
    if ms <= 0 {
        return "0s".to_string();
    }
    let total_secs = ms / 1000;
    let hours = total_secs / 3600;
    let minutes = (total_secs % 3600) / 60;
    let secs = total_secs % 60;

    if hours > 0 {
        format!("{}h {}m {}s", hours, minutes, secs)
    } else if minutes > 0 {
        format!("{}m {}s", minutes, secs)
    } else {
        format!("{}s", secs)
    }
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
    let had_cursor_cache = has_cursor_usage_cache_for_report(&home_dir);
    let explicit_cursor_filter = client_filter_explicitly_requests_cursor(&clients);
    let cursor_sync_result = auto_sync_cursor_for_local_report(&home_dir, &clients);
    let cursor_setup_warnings = setup_warnings_for_report(&home_dir, &clients);

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
                home_dir: home_dir.clone(),
                use_env_roots,
                clients,
                since,
                until,
                year,
                group_by: GroupBy::default(),
                scanner_settings: tui::settings::load_scanner_settings_for_home(&home_dir),
            })
            .await
        })
        .map_err(|e| anyhow::anyhow!(e))?;
    emit_cursor_sync_warning(
        cursor_sync_result.as_ref(),
        had_cursor_cache,
        explicit_cursor_filter,
    );
    emit_cursor_setup_warnings(&cursor_setup_warnings);

    let processing_time_ms = start.elapsed().as_millis() as u32;
    let output_data = to_ts_token_contribution_data(&graph_result, None);
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
                    if had_cursor_cache {
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

/// A client row dropped from a submission because it carried cost without any
/// token attribution. See [`exclude_tokenless_cost_contributions`].
#[derive(Debug, Clone, PartialEq)]
struct ExcludedTokenlessRow {
    date: String,
    client: String,
    model_id: String,
    provider_id: String,
    cost: f64,
}

fn client_token_total(tokens: &tokscale_core::TokenBreakdown) -> i64 {
    tokens.input + tokens.output + tokens.cache_read + tokens.cache_write + tokens.reasoning
}

/// Cursor's pre-2025-05 exports include `premium-tool-call` rows billed per
/// tool invocation with no token attribution. The server grandfathers these
/// (cost > 0, tokens = 0) rather than rejecting them, so the client must not
/// drop them either — otherwise that legitimate cost silently disappears from
/// the submission. Keep in sync with `CURSOR_LEGACY_TOKENLESS_MODELS` in
/// packages/frontend/src/lib/validation/submission.ts.
fn is_legacy_tokenless_cursor_row(client: &tokscale_core::ClientContribution) -> bool {
    client.client == "cursor"
        && client.model_id == "premium-tool-call"
        && client_token_total(&client.tokens) == 0
}

fn is_aggregate_only_warp_row(client: &tokscale_core::ClientContribution) -> bool {
    client.client == "warp"
        && client.model_id == "aggregate-requests"
        && client_token_total(&client.tokens) == 0
}

/// A row the server's "Cost submitted without tokens" sanity check would
/// reject: real cost with every token bucket at zero, excluding the Cursor
/// `premium-tool-call` carve-out above.
fn is_tokenless_costed_row(client: &tokscale_core::ClientContribution) -> bool {
    (is_aggregate_only_warp_row(client) || client.cost > 0.0)
        && client_token_total(&client.tokens) == 0
        && !is_legacy_tokenless_cursor_row(client)
}

/// Drop client rows that report cost without any tokens so the submission
/// passes the server's cost-without-tokens validation instead of being
/// rejected wholesale.
///
/// Cursor's usage export lists historical request/On-Demand charges (e.g.
/// `auto`, `claude-3.5-sonnet`, `o3`) with empty token columns, and Warp/Oz
/// only exposes aggregate request/spend counters. The server rejects cost with
/// no tokens, and request counts must not be submitted as fabricated tokens, so
/// we exclude the offending rows here and report them to the user.
///
/// Excluded rows always carry zero tokens, so only cost/messages change; token
/// totals, breakdowns, and intensities are untouched. Summary and year rollups
/// are recomputed from the trimmed contributions.
fn exclude_tokenless_cost_contributions(
    graph_result: &mut tokscale_core::GraphResult,
) -> Vec<ExcludedTokenlessRow> {
    let mut excluded: Vec<ExcludedTokenlessRow> = Vec::new();

    for day in graph_result.contributions.iter_mut() {
        let date = day.date.clone();
        let mut removed_cost = 0.0;
        let mut removed_messages: i32 = 0;

        day.clients.retain(|client| {
            if is_tokenless_costed_row(client) {
                excluded.push(ExcludedTokenlessRow {
                    date: date.clone(),
                    client: client.client.clone(),
                    model_id: client.model_id.clone(),
                    provider_id: client.provider_id.clone(),
                    cost: client.cost,
                });
                removed_cost += client.cost;
                removed_messages = removed_messages.saturating_add(client.messages);
                false
            } else {
                true
            }
        });

        if removed_cost > 0.0 || removed_messages > 0 {
            day.totals.cost = (day.totals.cost - removed_cost).max(0.0);
            day.totals.messages = day.totals.messages.saturating_sub(removed_messages).max(0);
        }
    }

    if !excluded.is_empty() {
        graph_result.summary = tokscale_core::calculate_summary(&graph_result.contributions);
        graph_result.years = tokscale_core::calculate_years(&graph_result.contributions);
    }

    excluded
}

/// Print the rows dropped by [`exclude_tokenless_cost_contributions`] so the
/// user can see exactly what was left out, capping the per-row detail so a long
/// history of legacy Cursor charges doesn't flood the terminal.
fn report_excluded_tokenless_rows(excluded: &[ExcludedTokenlessRow]) {
    use colored::Colorize;

    if excluded.is_empty() {
        return;
    }

    const MAX_DETAIL_ROWS: usize = 20;
    let total_cost: f64 = excluded.iter().map(|row| row.cost).sum();

    println!(
        "{}",
        format!(
            "  Excluded {} aggregate/cost-only row(s) with no token data:",
            excluded.len()
        )
        .yellow()
    );

    for row in excluded.iter().take(MAX_DETAIL_ROWS) {
        let provider = if row.provider_id.is_empty() {
            String::new()
        } else {
            format!(" (provider={})", row.provider_id)
        };
        println!(
            "{}",
            format!(
                "    - {}/{}{} on {}: ${:.4}",
                row.client, row.model_id, provider, row.date, row.cost
            )
            .bright_black()
        );
    }

    if excluded.len() > MAX_DETAIL_ROWS {
        println!(
            "{}",
            format!("    ... and {} more", excluded.len() - MAX_DETAIL_ROWS).bright_black()
        );
    }

    println!(
        "{}",
        format!(
            "    Excluded {} total; the rest is submitted.",
            format_currency(total_cost)
        )
        .bright_black()
    );
    println!();
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

    let auth_token = match auth::resolve_api_token() {
        Some(token) => token,
        None => {
            eprintln!("\n  {}", "Not logged in.".yellow());
            eprintln!(
                "{}",
                "  Run 'bunx tokscale@latest login' or set TOKSCALE_API_TOKEN.\n".bright_black()
            );
            std::process::exit(1);
        }
    };

    if auth_token.source == auth::ApiTokenSource::StoredCredentials
        && std::io::stdin().is_terminal()
        && std::io::stdout().is_terminal()
    {
        if let Some(username) = auth_token.username.as_deref() {
            let _ = prompt_star_repo(username);
        }
    }

    println!("\n  {}\n", "Tokscale - Submit Usage Data".cyan());

    let explicit_cursor_filter = client_filter_explicitly_requests_cursor(&clients);
    let explicit_warp_filter = client_filter_explicitly_requests_warp(&clients);
    let clients = clients.or_else(|| Some(default_submit_clients()));

    let include_cursor = clients
        .as_ref()
        .is_none_or(|s| s.iter().any(|src| src == "cursor"));
    let report_home: Option<String> = None;
    let has_cursor_cache = has_cursor_usage_cache_for_report(&report_home);
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
    if explicit_cursor_filter || explicit_warp_filter {
        let cursor_setup_warnings = setup_warnings_for_report(&report_home, &clients);
        emit_cursor_setup_warnings(&cursor_setup_warnings);
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

    // Drop cost-only rows the server would reject (Cursor historical exports
    // record per-request cost with empty token columns) and report what was
    // left out, so a single legacy charge can't block the whole submission.
    let excluded_rows = exclude_tokenless_cost_contributions(&mut graph_result);
    report_excluded_tokenless_rows(&excluded_rows);

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

    let submit_device = device::resolve_submit_device()?;
    let submit_payload = to_ts_token_contribution_data(&graph_result, Some(&submit_device));

    let response = rt.block_on(async {
        reqwest::Client::new()
            .post(format!("{}/api/submit", api_url))
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {}", auth_token.token))
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
            if let Some(username) = body
                .username
                .clone()
                .or_else(|| auth_token.username.clone())
            {
                println!();
                println!(
                    "{}",
                    osc8_link_with_text(
                        &format!("{}/u/{}", api_url, username),
                        &format!("  View your profile: {}/u/{}", api_url, username),
                    )
                    .cyan()
                );
                println!();
            }

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
    use crate::tui::{save_cached_data, CacheReportScope, DataLoader};

    // The TUI cache key includes date filters, but not `--home`. Writing
    // home-scoped data would still poison the default cache, so keep that
    // guard until home is part of the cache key.
    if home_dir.is_some() {
        eprintln!(
            "tokscale: --write-cache skipped because --home is set; \
             the TUI cache key does not include that filter and writing would poison future TUI launches."
        );
        return;
    }

    let enabled_set = resolve_light_cache_filter_set(clients);
    let scan_clients: Vec<tokscale_core::ClientId> = enabled_set
        .iter()
        .filter_map(|filter| filter.to_client_id())
        .collect();
    let include_synthetic = enabled_set.contains(&ClientFilter::Synthetic);

    // Cache writes are best-effort: the report has already been flushed
    // to stdout by the time we reach here, so a scan failure from the
    // background loader must NOT propagate up and turn a successful
    // user-visible report into a non-zero exit code. Mirrors the
    // pattern in `run_warm_tui_cache` below.
    let loader = DataLoader::with_filters(None, since.clone(), until.clone(), year.clone());
    let report_scope = CacheReportScope::new(since.clone(), until.clone(), year.clone());
    if let Ok(data) = loader.load(&scan_clients, group_by, include_synthetic) {
        save_cached_data(&data, &enabled_set, group_by, &report_scope);
    }
}

fn run_warm_tui_cache() -> Result<()> {
    use crate::tui::{save_cached_data, CacheReportScope, DataLoader, TUI_DEFAULT_GROUP_BY};
    use tokscale_core::ClientId;

    // Warm the cache using the same default filter set the TUI uses on
    // a no-flag launch. Going through `resolve_default_tui_filter_set()`
    // keeps these two paths in lockstep — including the user's
    // `defaultClients` setting, which the TUI honors via
    // `build_client_filter`. If they drift, every TUI launch after
    // `submit` becomes a cache miss instead of a fresh hit, defeating
    // the warming.
    //
    // The `group_by` MUST be `TUI_DEFAULT_GROUP_BY`, NOT
    // `GroupBy::default()`. Using `GroupBy::default()` here is the bug
    // that motivated this constant — the TUI's cache reader keys on
    // `TUI_DEFAULT_GROUP_BY` (= `GroupBy::Model`) while
    // `GroupBy::default()` is `GroupBy::ClientModel`, so the warm cache
    // was written under a key the TUI never queried. Every submit
    // silently invalidated the next TUI launch.
    let enabled_set = resolve_default_tui_filter_set();
    let scan_clients: Vec<ClientId> = enabled_set
        .iter()
        .filter_map(|f| f.to_client_id())
        .collect();
    let include_synthetic = enabled_set.contains(&ClientFilter::Synthetic);
    let loader = DataLoader::with_filters(None, None, None, None);
    if let Ok(data) = loader.load(&scan_clients, &TUI_DEFAULT_GROUP_BY, include_synthetic) {
        save_cached_data(
            &data,
            &enabled_set,
            &TUI_DEFAULT_GROUP_BY,
            &CacheReportScope::default(),
        );
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

fn run_codex_command(subcommand: CodexSubcommand) -> Result<()> {
    match subcommand {
        CodexSubcommand::Import { name } => commands::usage::codex::run_codex_import(name),
        CodexSubcommand::Accounts { json } => commands::usage::codex::run_codex_accounts(json),
        CodexSubcommand::Switch { name } => commands::usage::codex::run_codex_switch(&name),
        CodexSubcommand::Remove { name } => commands::usage::codex::run_codex_remove(&name),
        CodexSubcommand::Status { name, json } => {
            commands::usage::codex::run_codex_status(name, json)
        }
    }
}

fn run_antigravity_command(subcommand: AntigravitySubcommand) -> Result<()> {
    match subcommand {
        AntigravitySubcommand::Sync => antigravity::run_antigravity_sync(),
        AntigravitySubcommand::Status { json } => antigravity::run_antigravity_status(json),
        AntigravitySubcommand::PurgeCache => antigravity::run_antigravity_purge_cache(),
    }
}

/// Parse `--variant` into a typed value.
///
/// Returns:
/// - `Ok(Some(v))` when a recognized value was provided
/// - `Ok(None)` when the flag was omitted entirely
/// - `Err` when an unrecognized value was provided
///
/// The earlier version returned `Option<_>` and merged the "unrecognized" and
/// "omitted" cases, which let callers silently fall through to "all variants"
/// when the user typed something like `--variant slo` — they got every variant
/// touched instead of an error.
fn parse_variant_arg(arg: Option<&str>) -> Result<Option<trae::auth::TraeVariant>> {
    match arg {
        Some("solo") => Ok(Some(trae::auth::TraeVariant::Solo)),
        Some("ide") => Ok(Some(trae::auth::TraeVariant::Ide)),
        Some(other) => anyhow::bail!("unknown variant: {other}, valid values: solo, ide"),
        None => Ok(None),
    }
}

fn run_trae_command(subcommand: TraeSubcommand) -> Result<()> {
    use colored::Colorize;
    let rt = tokio::runtime::Runtime::new()?;

    match subcommand {
        TraeSubcommand::Login { manual, variant } => {
            if manual {
                use std::io::{self, Write};
                // Default to international Solo when `--variant` is omitted.
                let selected =
                    parse_variant_arg(variant.as_deref())?.unwrap_or(trae::auth::TraeVariant::Solo);
                println!();
                println!("  {}", "Trae Manual Token Login".cyan());
                println!(
                    "  {}",
                    "Paste your JWT access token from the browser DevTools:".bright_black()
                );
                println!(
                    "  {}",
                    "1. Open https://www.trae.ai/account-setting#usage".bright_black()
                );
                println!(
                    "  {}",
                    "2. F12 → Network → filter 'query_user_usage' → copy Authorization value"
                        .bright_black()
                );
                print!("  Token: ");
                io::stdout().flush()?;
                let mut token = String::new();
                io::stdin().read_line(&mut token)?;
                let token = token.trim().to_string();
                if token.is_empty() {
                    anyhow::bail!("token must not be empty");
                }
                trae::auth::save_manual_token(selected, token, None)?;
                println!(
                    "\n  {}",
                    format!("Token saved for {}", selected.client_str()).green()
                );
            } else {
                let variants: Vec<trae::auth::TraeVariant> =
                    match parse_variant_arg(variant.as_deref())? {
                        Some(v) => vec![v],
                        None => trae::auth::all_variants().to_vec(),
                    };

                let mut any_success = false;
                for v in variants {
                    match rt.block_on(trae::auth::resolve_token(v)) {
                        Ok(_) => {
                            println!("  {} logged in (auto-detected)", v.client_str().green());
                            any_success = true;
                        }
                        Err(e) => {
                            println!("  {} auto-login failed: {}", v.client_str().yellow(), e);
                        }
                    }
                }
                if !any_success {
                    println!(
                        "  {}",
                        "No Trae credentials found. Use --manual to paste a token by hand."
                            .yellow()
                    );
                }
            }
            Ok(())
        }
        TraeSubcommand::Logout { variant } => {
            let variants: Vec<trae::auth::TraeVariant> =
                match parse_variant_arg(variant.as_deref())? {
                    Some(v) => vec![v],
                    None => trae::auth::all_variants().to_vec(),
                };
            for v in variants {
                trae::auth::logout(v)?;
                println!("  {} logged out", v.client_str().green());
            }
            Ok(())
        }
        TraeSubcommand::Status { json } => {
            let mut status = serde_json::Map::new();
            for v in trae::auth::all_variants() {
                let has = trae::auth::has_credentials(v);
                if json {
                    status.insert(v.client_str().to_string(), serde_json::Value::Bool(has));
                } else {
                    println!(
                        "  {}: {}",
                        v.client_str(),
                        if has {
                            "authenticated".green()
                        } else {
                            "not authenticated".yellow()
                        }
                    );
                }
            }
            if json {
                println!("{}", serde_json::to_string_pretty(&status)?);
            }
            Ok(())
        }
        TraeSubcommand::Sync { since, include_aux } => {
            let days = since.unwrap_or(30);
            // Negative `days` would compute `now - (negative * 86400)` → a
            // future `start_time`, and zero collapses the query window to an
            // empty range. Reject both at the CLI boundary instead of
            // forwarding garbage to the sync layer.
            if days <= 0 {
                anyhow::bail!("--since must be a positive number of days (got {days})");
            }
            // Trae IDE and Trae Solo share account-level usage data, so we
            // always sync once using whichever credential source is available.
            let variants: Vec<trae::auth::TraeVariant> = trae::auth::all_variants()
                .into_iter()
                .filter(|v| trae::auth::has_credentials(*v))
                .collect();
            rt.block_on(trae::sync::run_trae_sync(&variants, days, include_aux))
        }
    }
}

fn run_warp_command(subcommand: WarpSubcommand) -> Result<()> {
    match subcommand {
        WarpSubcommand::Login { token, cookie } => warp::run_warp_login(token, cookie),
        WarpSubcommand::Logout { purge_cache } => warp::run_warp_logout(purge_cache),
        WarpSubcommand::Status { json } => warp::run_warp_status(json),
        WarpSubcommand::Sync { json } => warp::run_warp_sync(json),
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

struct CaptureCommandOutcome {
    exit_code: i32,
    timed_out: bool,
}

fn run_capture_command(
    command: &str,
    args: &[String],
    output_path: &Path,
    timeout: Duration,
) -> Result<CaptureCommandOutcome> {
    use std::io::{Read, Write};
    use std::process::Command;
    use std::thread;
    use std::time::Instant;

    let mut child = Command::new(command)
        .args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::inherit())
        .stdin(std::process::Stdio::inherit())
        .spawn()
        .map_err(|e| anyhow::anyhow!("Failed to spawn '{}': {}", command, e))?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow::anyhow!("Failed to capture stdout from command"))?;

    let mut output_file = std::fs::File::create(output_path).map_err(|e| {
        anyhow::anyhow!(
            "Failed to create output file '{}': {}",
            output_path.display(),
            e
        )
    })?;

    let output_handle = thread::spawn(move || -> Result<()> {
        let mut reader = std::io::BufReader::new(stdout);
        let mut buffer = [0; 8192];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => return Ok(()),
                Ok(n) => output_file
                    .write_all(&buffer[..n])
                    .map_err(|e| anyhow::anyhow!("Failed to write to output file: {}", e))?,
                Err(e) => {
                    return Err(anyhow::anyhow!(
                        "Failed to read from subprocess stdout: {}",
                        e
                    ));
                }
            }
        }
    });

    let deadline = Instant::now() + timeout;
    let mut timed_out = false;
    let status = loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|e| anyhow::anyhow!("Failed to wait for subprocess: {}", e))?
        {
            break status;
        }

        if Instant::now() >= deadline {
            timed_out = true;
            let _ = child.kill();
            break child
                .wait()
                .map_err(|e| anyhow::anyhow!("Failed to wait for timed-out subprocess: {}", e))?;
        }

        thread::sleep(Duration::from_millis(25));
    };

    let output_result = output_handle
        .join()
        .map_err(|_| anyhow::anyhow!("Subprocess stdout reader thread panicked"))?;
    if !timed_out {
        output_result?;
    }

    Ok(CaptureCommandOutcome {
        exit_code: status.code().unwrap_or(1),
        timed_out,
    })
}

fn run_headless_command(
    source: &str,
    args: Vec<String>,
    format: Option<String>,
    output: Option<String>,
    no_auto_flags: bool,
) -> Result<()> {
    use chrono::Utc;
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

    let outcome =
        run_capture_command(&source_lower, &final_args, Path::new(&output_path), timeout)?;

    if outcome.timed_out {
        eprintln!(
            "{}",
            format!("\n  Subprocess timed out after {}s", timeout.as_secs()).red()
        );
        eprintln!("{}", "  Partial output saved. Increase timeout with TOKSCALE_NATIVE_TIMEOUT_MS or settings.json".bright_black());
        println!();
        std::process::exit(124);
    }

    println!(
        "{}",
        format!("✓ Saved headless output to {}", output_path).green()
    );
    println!();

    if outcome.exit_code != 0 {
        std::process::exit(outcome.exit_code);
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

    #[test]
    fn test_parse_variant_arg_accepts_known_values() {
        assert_eq!(
            parse_variant_arg(Some("solo")).unwrap(),
            Some(trae::auth::TraeVariant::Solo)
        );
        assert_eq!(
            parse_variant_arg(Some("ide")).unwrap(),
            Some(trae::auth::TraeVariant::Ide)
        );
    }

    #[test]
    fn test_parse_variant_arg_none_when_omitted() {
        assert_eq!(parse_variant_arg(None).unwrap(), None);
    }

    #[test]
    fn test_parse_variant_arg_rejects_unknown_value() {
        // The earlier `Option`-returning version converted this to `None`
        // and the caller fell through to "all variants" — a typo like
        // `--variant slo` would log out every variant. Now we error out.
        let err = parse_variant_arg(Some("slo")).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("unknown variant"), "got: {msg}");
        assert!(msg.contains("slo"), "got: {msg}");
    }

    #[test]
    fn test_parse_variant_arg_rejects_empty_string() {
        assert!(parse_variant_arg(Some("")).is_err());
    }

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
            active_time_ms: None,
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
            time_metrics: None,
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

    /// The 32 per-client boolean flags removed in 4.0.0. After removal every
    /// one of these must produce a clap parse error — backward-compat parsing
    /// is intentionally gone (breaking change). Keep this list in sync with the
    /// flags deleted from `ClientFlags`.
    const REMOVED_LEGACY_CLIENT_FLAGS: [&str; 32] = [
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
        "zed",
        "kiro",
        "trae",
        "warp",
        "cline",
        "gjc",
        "grok",
        "jcode",
        "commandcode",
        "micode",
        "synthetic",
    ];

    #[test]
    fn test_removed_legacy_client_flags_now_error() {
        for flag in REMOVED_LEGACY_CLIENT_FLAGS {
            let arg = format!("--{flag}");
            let result = Cli::try_parse_from(["tokscale", arg.as_str()]);
            assert!(
                result.is_err(),
                "expected `{arg}` to be rejected after removal, but it parsed"
            );
        }
    }

    #[test]
    fn test_canonical_client_still_parses_for_removed_flag_names() {
        // Every removed boolean flag name remains a valid `--client` value.
        for flag in REMOVED_LEGACY_CLIENT_FLAGS {
            let cli = Cli::try_parse_from(["tokscale", "--client", flag])
                .unwrap_or_else(|_| panic!("`--client {flag}` should parse"));
            assert_eq!(
                build_client_filter_with_defaults(cli.clients, &[]),
                Some(vec![flag.to_string()]),
                "`--client {flag}` should resolve to a single source"
            );
        }
    }

    #[test]
    fn test_canonical_client_parses_single_and_multi() {
        let cli = Cli::try_parse_from(["tokscale", "--client", "opencode"]).expect("parse ok");
        assert_eq!(
            build_client_filter_with_defaults(cli.clients, &[]),
            Some(vec!["opencode".to_string()])
        );

        let cli =
            Cli::try_parse_from(["tokscale", "--client", "opencode,claude"]).expect("parse ok");
        assert_eq!(
            build_client_filter_with_defaults(cli.clients, &[]),
            Some(vec!["opencode".to_string(), "claude".to_string()])
        );

        let cli = Cli::try_parse_from(["tokscale", "--client", "synthetic"]).expect("parse ok");
        assert_eq!(
            build_client_filter_with_defaults(cli.clients, &[]),
            Some(vec!["synthetic".to_string()])
        );
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
    fn test_client_filter_gjc_round_trip() {
        use tokscale_core::ClientId;
        // gjc parses as the canonical lowercase id and round-trips through
        // both the ClientId<->ClientFilter conversions and the id string.
        assert_eq!(ClientFilter::Gjc.as_filter_str(), "gjc");
        assert_eq!(ClientFilter::Gjc.to_client_id(), Some(ClientId::Gjc));
        assert_eq!(
            ClientFilter::from_client_id(ClientId::Gjc),
            ClientFilter::Gjc
        );
        assert_eq!(ClientFilter::Gjc.as_filter_str(), ClientId::Gjc.as_str());
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
        };
        let defaults = vec!["opencode".to_string(), "claude".to_string()];
        assert_eq!(
            build_client_filter_with_defaults(flags, &defaults),
            Some(vec!["codex".to_string()])
        );
    }

    #[test]
    fn test_build_client_filter_canonical_flag_overrides_defaults() {
        // A canonical `--client` value counts as "user passed something" →
        // defaults ignored. CLI flags always win over settings.json.
        let flags = ClientFlags {
            clients: vec![ClientFilter::Opencode],
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
    fn test_wrapped_parses_clients_view_flag() {
        let cli = Cli::try_parse_from(["tokscale", "wrapped"]).expect("parse ok");
        let Some(Commands::Wrapped {
            show_clients,
            agents,
            ..
        }) = cli.command
        else {
            panic!("expected wrapped command");
        };
        assert!(!show_clients);
        assert!(!agents);

        let cli = Cli::try_parse_from(["tokscale", "wrapped", "--clients"]).expect("parse ok");
        let Some(Commands::Wrapped { show_clients, .. }) = cli.command else {
            panic!("expected wrapped command");
        };
        assert!(show_clients);
    }

    #[test]
    fn test_wrapped_client_filter_coexists_with_clients_view_flag() {
        let cli =
            Cli::try_parse_from(["tokscale", "wrapped", "--client", "opencode"]).expect("parse ok");
        let Some(Commands::Wrapped {
            client_flags,
            show_clients,
            ..
        }) = cli.command
        else {
            panic!("expected wrapped command");
        };
        assert_eq!(client_flags.clients, vec![ClientFilter::Opencode]);
        assert!(!show_clients);

        let cli = Cli::try_parse_from(["tokscale", "wrapped", "--clients", "--client", "opencode"])
            .expect("parse ok");
        let Some(Commands::Wrapped {
            client_flags,
            show_clients,
            ..
        }) = cli.command
        else {
            panic!("expected wrapped command");
        };
        assert_eq!(client_flags.clients, vec![ClientFilter::Opencode]);
        assert!(show_clients);
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
    fn test_default_submit_clients_excludes_crush() {
        let clients = default_submit_clients();
        assert!(clients.contains(&"synthetic".to_string()));
        assert!(clients.contains(&"zed".to_string()));
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
    fn test_client_filter_zed_round_trip() {
        assert_eq!(
            ClientFilter::from_filter_str("zed"),
            Some(ClientFilter::Zed)
        );
        assert_eq!(ClientFilter::Zed.as_filter_str(), "zed");
        assert_eq!(
            ClientFilter::Zed.to_client_id(),
            Some(tokscale_core::ClientId::Zed)
        );
        assert_eq!(
            ClientFilter::from_client_id(tokscale_core::ClientId::Zed),
            ClientFilter::Zed
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
    fn test_login_token_option_parses() {
        let cli = Cli::try_parse_from(["tokscale", "login", "--token", "tt_ci_token"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Commands::Login {
                token: Some(token)
            }) if token == "tt_ci_token"
        ));
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
        let (since, until) = build_date_filter(&DateRangeFlags {
            since: Some("2024-01-01".to_string()),
            until: Some("2024-12-31".to_string()),
            ..DateRangeFlags::default()
        });
        assert_eq!(since, Some("2024-01-01".to_string()));
        assert_eq!(until, Some("2024-12-31".to_string()));
    }

    #[test]
    fn test_build_date_filter_no_filters() {
        let (since, until) = build_date_filter(&DateRangeFlags::default());
        assert_eq!(since, None);
        assert_eq!(until, None);
    }

    #[test]
    fn test_build_date_filter_today_uses_provided_local_date() {
        let today = chrono::NaiveDate::from_ymd_opt(2026, 3, 8).unwrap();
        let (since, until) = build_date_filter_for_date(
            &DateRangeFlags {
                today: true,
                ..DateRangeFlags::default()
            },
            today,
        );
        assert_eq!(since, Some("2026-03-08".to_string()));
        assert_eq!(until, Some("2026-03-08".to_string()));
    }

    #[test]
    fn test_build_date_filter_yesterday_uses_provided_local_date() {
        let today = chrono::NaiveDate::from_ymd_opt(2026, 3, 8).unwrap();
        let (since, until) = build_date_filter_for_date(
            &DateRangeFlags {
                yesterday: true,
                ..DateRangeFlags::default()
            },
            today,
        );
        assert_eq!(since, Some("2026-03-07".to_string()));
        assert_eq!(until, Some("2026-03-07".to_string()));
    }

    #[test]
    fn test_build_date_filter_week_uses_provided_local_date() {
        let today = chrono::NaiveDate::from_ymd_opt(2026, 3, 8).unwrap();
        let (since, until) = build_date_filter_for_date(
            &DateRangeFlags {
                week: true,
                ..DateRangeFlags::default()
            },
            today,
        );
        assert_eq!(since, Some("2026-03-02".to_string()));
        assert_eq!(until, Some("2026-03-08".to_string()));
    }

    #[test]
    fn test_build_date_filter_month_uses_provided_local_date() {
        let today = chrono::NaiveDate::from_ymd_opt(2026, 3, 8).unwrap();
        let (since, until) = build_date_filter_for_date(
            &DateRangeFlags {
                month: true,
                ..DateRangeFlags::default()
            },
            today,
        );
        assert_eq!(since, Some("2026-03-01".to_string()));
        assert_eq!(until, Some("2026-03-08".to_string()));
    }

    #[test]
    fn test_normalize_year_filter_with_year() {
        let year = normalize_year_filter(&DateRangeFlags {
            year: Some("2024".to_string()),
            ..DateRangeFlags::default()
        });
        assert_eq!(year, Some("2024".to_string()));
    }

    #[test]
    fn test_normalize_year_filter_with_today() {
        let year = normalize_year_filter(&DateRangeFlags {
            today: true,
            year: Some("2024".to_string()),
            ..DateRangeFlags::default()
        });
        assert_eq!(year, None);
    }

    #[test]
    fn test_normalize_year_filter_with_yesterday() {
        let year = normalize_year_filter(&DateRangeFlags {
            yesterday: true,
            year: Some("2024".to_string()),
            ..DateRangeFlags::default()
        });
        assert_eq!(year, None);
    }

    #[test]
    fn test_normalize_year_filter_with_week() {
        let year = normalize_year_filter(&DateRangeFlags {
            week: true,
            year: Some("2024".to_string()),
            ..DateRangeFlags::default()
        });
        assert_eq!(year, None);
    }

    #[test]
    fn test_normalize_year_filter_with_month() {
        let year = normalize_year_filter(&DateRangeFlags {
            month: true,
            year: Some("2024".to_string()),
            ..DateRangeFlags::default()
        });
        assert_eq!(year, None);
    }

    #[test]
    fn test_normalize_year_filter_no_year() {
        let year = normalize_year_filter(&DateRangeFlags::default());
        assert_eq!(year, None);
    }

    /// Parses `args` expecting failure; panics if parsing unexpectedly
    /// succeeds. Avoids `unwrap_err()` since `Cli` does not derive `Debug`.
    fn expect_parse_error(args: &[&str]) -> clap::Error {
        match Cli::try_parse_from(args) {
            Ok(_) => panic!("expected `{}` to fail to parse", args.join(" ")),
            Err(err) => err,
        }
    }

    #[test]
    fn test_date_shortcut_flags_conflict() {
        let err = expect_parse_error(&["tokscale", "--today", "--yesterday"]);
        assert_eq!(err.kind(), clap::error::ErrorKind::ArgumentConflict);

        let err = expect_parse_error(&["tokscale", "--week", "--month"]);
        assert_eq!(err.kind(), clap::error::ErrorKind::ArgumentConflict);
    }

    #[test]
    fn test_date_shortcut_conflicts_with_since_until_year() {
        let err = expect_parse_error(&["tokscale", "--today", "--since", "2024-01-01"]);
        assert_eq!(err.kind(), clap::error::ErrorKind::ArgumentConflict);

        let err = expect_parse_error(&["tokscale", "--week", "--until", "2024-12-31"]);
        assert_eq!(err.kind(), clap::error::ErrorKind::ArgumentConflict);

        let err = expect_parse_error(&["tokscale", "--month", "--year", "2024"]);
        assert_eq!(err.kind(), clap::error::ErrorKind::ArgumentConflict);
    }

    #[test]
    fn test_date_shortcut_conflict_applies_to_subcommands() {
        let err = expect_parse_error(&["tokscale", "models", "--today", "--yesterday"]);
        assert_eq!(err.kind(), clap::error::ErrorKind::ArgumentConflict);
    }

    #[test]
    fn test_since_until_year_still_combine() {
        let cli = Cli::try_parse_from([
            "tokscale",
            "--since",
            "2024-01-01",
            "--until",
            "2024-12-31",
            "--year",
            "2024",
        ])
        .unwrap();
        assert_eq!(cli.date.since.as_deref(), Some("2024-01-01"));
        assert_eq!(cli.date.until.as_deref(), Some("2024-12-31"));
        assert_eq!(cli.date.year.as_deref(), Some("2024"));
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
    fn test_capitalize_client_jcode() {
        assert_eq!(capitalize_client("jcode"), "Jcode");
    }

    #[test]
    fn test_capitalize_client_unknown() {
        assert_eq!(capitalize_client("unknown"), "unknown");
    }

    #[test]
    fn test_get_date_range_label_today() {
        let label = get_date_range_label(&DateRangeFlags {
            today: true,
            ..DateRangeFlags::default()
        });
        assert_eq!(label, Some("Today".to_string()));
    }

    #[test]
    fn test_get_date_range_label_yesterday() {
        let label = get_date_range_label(&DateRangeFlags {
            yesterday: true,
            ..DateRangeFlags::default()
        });
        assert_eq!(label, Some("Yesterday".to_string()));
    }

    #[test]
    fn test_get_date_range_label_week() {
        let label = get_date_range_label(&DateRangeFlags {
            week: true,
            ..DateRangeFlags::default()
        });
        assert_eq!(label, Some("Last 7 days".to_string()));
    }

    #[test]
    fn test_get_date_range_label_month_uses_provided_local_date() {
        let today = chrono::NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
        let label = get_date_range_label_for_date(
            &DateRangeFlags {
                month: true,
                ..DateRangeFlags::default()
            },
            today,
        );
        assert_eq!(label, Some("March 2026".to_string()));
    }

    #[test]
    fn test_get_date_range_label_year() {
        let label = get_date_range_label(&DateRangeFlags {
            year: Some("2024".to_string()),
            ..DateRangeFlags::default()
        });
        assert_eq!(label, Some("2024".to_string()));
    }

    #[test]
    fn test_get_date_range_label_custom_since() {
        let label = get_date_range_label(&DateRangeFlags {
            since: Some("2024-01-01".to_string()),
            ..DateRangeFlags::default()
        });
        assert_eq!(label, Some("from 2024-01-01".to_string()));
    }

    #[test]
    fn test_get_date_range_label_custom_until() {
        let label = get_date_range_label(&DateRangeFlags {
            until: Some("2024-12-31".to_string()),
            ..DateRangeFlags::default()
        });
        assert_eq!(label, Some("to 2024-12-31".to_string()));
    }

    #[test]
    fn test_get_date_range_label_custom_range() {
        let label = get_date_range_label(&DateRangeFlags {
            since: Some("2024-01-01".to_string()),
            until: Some("2024-12-31".to_string()),
            ..DateRangeFlags::default()
        });
        assert_eq!(label, Some("from 2024-01-01 to 2024-12-31".to_string()));
    }

    #[test]
    fn test_get_date_range_label_none() {
        let label = get_date_range_label(&DateRangeFlags::default());
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

    fn client_contribution(
        client: &str,
        model_id: &str,
        provider_id: &str,
        total_tokens: i64,
        cost: f64,
        messages: i32,
    ) -> ClientContribution {
        ClientContribution {
            client: client.to_string(),
            model_id: model_id.to_string(),
            provider_id: provider_id.to_string(),
            tokens: token_breakdown(total_tokens),
            cost,
            messages,
        }
    }

    fn day_with_clients(
        date: &str,
        token_breakdown_total: i64,
        clients: Vec<ClientContribution>,
    ) -> DailyContribution {
        let tokens: i64 = clients.iter().map(|c| client_token_total(&c.tokens)).sum();
        let cost: f64 = clients.iter().map(|c| c.cost).sum();
        let messages: i32 = clients.iter().map(|c| c.messages).sum();
        DailyContribution {
            date: date.to_string(),
            totals: DailyTotals {
                tokens,
                cost,
                messages,
            },
            intensity: 0,
            token_breakdown: token_breakdown(token_breakdown_total),
            clients,
            active_time_ms: None,
        }
    }

    #[test]
    fn test_exclude_tokenless_cost_drops_offenders_and_keeps_the_rest() {
        // A token-bearing row shares the day with a tokenless cursor charge
        // (cost, no tokens) and a grandfathered premium-tool-call row.
        let mut graph = graph_result_with_contributions(vec![day_with_clients(
            "2025-05-28",
            100,
            vec![
                client_contribution("cursor", "claude-3.7-sonnet", "anthropic", 100, 0.03, 1),
                client_contribution("cursor", "auto", "cursor", 0, 0.04, 1),
                client_contribution("cursor", "premium-tool-call", "cursor", 0, 2.05, 44),
            ],
        )]);

        let excluded = exclude_tokenless_cost_contributions(&mut graph);

        // Only the tokenless `auto` row is dropped.
        assert_eq!(excluded.len(), 1);
        assert_eq!(excluded[0].model_id, "auto");
        assert!((excluded[0].cost - 0.04).abs() < 1e-9);

        let day = &graph.contributions[0];
        assert_eq!(day.clients.len(), 2);
        assert!(day.clients.iter().all(|c| c.model_id != "auto"));
        // premium-tool-call is preserved (server carve-out).
        assert!(day
            .clients
            .iter()
            .any(|c| c.model_id == "premium-tool-call"));
        // Tokens untouched; cost/messages reduced by the dropped row only.
        assert_eq!(day.totals.tokens, 100);
        assert!((day.totals.cost - 2.08).abs() < 1e-9);
        assert_eq!(day.totals.messages, 45);
        assert!((graph.summary.total_cost - 2.08).abs() < 1e-9);
        assert_eq!(graph.summary.total_tokens, 100);
    }

    #[test]
    fn test_exclude_tokenless_cost_zeroes_a_fully_tokenless_day() {
        let mut graph = graph_result_with_contributions(vec![day_with_clients(
            "2025-05-30",
            0,
            vec![
                client_contribution("cursor", "auto", "cursor", 0, 0.04, 1),
                client_contribution("cursor", "auto", "cursor", 0, 0.04, 1),
            ],
        )]);

        let excluded = exclude_tokenless_cost_contributions(&mut graph);

        assert_eq!(excluded.len(), 2);
        let day = &graph.contributions[0];
        assert!(day.clients.is_empty());
        assert_eq!(day.totals.cost, 0.0);
        assert_eq!(day.totals.tokens, 0);
        assert_eq!(graph.summary.total_cost, 0.0);
    }

    #[test]
    fn test_exclude_tokenless_cost_is_noop_without_offenders() {
        let mut graph = graph_result_with_contributions(vec![day_with_clients(
            "2025-05-28",
            100,
            vec![
                client_contribution("codex", "gpt-5", "openai", 100, 0.03, 1),
                // Grandfathered cursor legacy row must not be dropped.
                client_contribution("cursor", "premium-tool-call", "cursor", 0, 2.05, 44),
            ],
        )]);
        let original_cost = graph.summary.total_cost;

        let excluded = exclude_tokenless_cost_contributions(&mut graph);

        assert!(excluded.is_empty());
        assert_eq!(graph.contributions[0].clients.len(), 2);
        assert_eq!(graph.summary.total_cost, original_cost);
    }

    #[test]
    fn test_exclude_tokenless_cost_drops_warp_aggregate_requests() {
        let mut graph = graph_result_with_contributions(vec![day_with_clients(
            "2026-01-02",
            0,
            vec![client_contribution(
                "warp",
                "aggregate-requests",
                "warp",
                0,
                12.34,
                42,
            )],
        )]);

        let excluded = exclude_tokenless_cost_contributions(&mut graph);

        assert_eq!(excluded.len(), 1);
        assert_eq!(excluded[0].client, "warp");
        assert_eq!(excluded[0].model_id, "aggregate-requests");
        assert!(graph.contributions[0].clients.is_empty());
        assert_eq!(graph.summary.total_tokens, 0);
        assert_eq!(graph.summary.total_cost, 0.0);
    }

    #[test]
    fn test_submit_payload_includes_device_when_provided() {
        let graph = graph_result_with_contributions(vec![daily_contribution(
            "2026-12-31",
            20,
            2.50,
            "codex",
            "model-b",
        )]);
        let device = device::SubmitDevice {
            id: "dev_test".to_string(),
            name: Some("Test device".to_string()),
        };

        let payload = to_ts_token_contribution_data(&graph, Some(&device));

        assert_eq!(payload.device.as_ref().unwrap().id, "dev_test");
        assert_eq!(
            payload.device.as_ref().unwrap().name.as_deref(),
            Some("Test device")
        );
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
    fn clap_accepts_codex_account_commands() {
        assert!(Cli::try_parse_from(["tokscale", "codex", "import", "--name", "work"]).is_ok());
        assert!(Cli::try_parse_from(["tokscale", "codex", "accounts"]).is_ok());
        assert!(Cli::try_parse_from(["tokscale", "codex", "accounts", "--json"]).is_ok());
        assert!(Cli::try_parse_from(["tokscale", "codex", "switch", "work"]).is_ok());
        assert!(Cli::try_parse_from(["tokscale", "codex", "remove", "work"]).is_ok());
        assert!(Cli::try_parse_from(["tokscale", "codex", "status"]).is_ok());
        assert!(Cli::try_parse_from(["tokscale", "codex", "status", "--name", "work"]).is_ok());
        assert!(
            Cli::try_parse_from(["tokscale", "codex", "status", "--name", "work", "--json"])
                .is_ok()
        );
    }

    #[test]
    fn clap_accepts_warp_status_and_sync_commands() {
        assert!(Cli::try_parse_from(["tokscale", "warp", "status"]).is_ok());
        assert!(Cli::try_parse_from(["tokscale", "warp", "status", "--json"]).is_ok());
        assert!(Cli::try_parse_from(["tokscale", "warp", "sync"]).is_ok());
        assert!(Cli::try_parse_from(["tokscale", "warp", "sync", "--json"]).is_ok());
    }

    #[test]
    fn client_filter_round_trips_warp() {
        assert_eq!(
            ClientFilter::from_filter_str("warp"),
            Some(ClientFilter::Warp)
        );
        assert_eq!(ClientFilter::Warp.as_filter_str(), "warp");
        assert_eq!(
            ClientFilter::Warp.to_client_id(),
            Some(tokscale_core::ClientId::Warp)
        );
        assert_eq!(
            ClientFilter::from_client_id(tokscale_core::ClientId::Warp),
            ClientFilter::Warp
        );
    }

    #[test]
    fn client_filter_round_trips_grok() {
        assert_eq!(
            ClientFilter::from_filter_str("grok"),
            Some(ClientFilter::Grok)
        );
        assert_eq!(ClientFilter::Grok.as_filter_str(), "grok");
        assert_eq!(
            ClientFilter::Grok.to_client_id(),
            Some(tokscale_core::ClientId::Grok)
        );
        assert_eq!(
            ClientFilter::from_client_id(tokscale_core::ClientId::Grok),
            ClientFilter::Grok
        );
    }

    #[test]
    fn default_submit_clients_excludes_warp_aggregate_source() {
        let clients = default_submit_clients();
        assert!(!clients.contains(&"warp".to_string()));
    }

    #[test]
    fn warp_setup_warning_explains_missing_aggregate_cache() {
        let temp = tempfile::TempDir::new().unwrap();
        let warnings = warp_setup_warnings_for_report(
            &Some(temp.path().to_string_lossy().to_string()),
            &Some(vec!["warp".to_string()]),
        );

        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("tokscale warp"));
        assert!(warnings[0].contains("does not infer tokens from request counts"));
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
