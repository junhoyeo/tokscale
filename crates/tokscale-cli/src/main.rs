mod auth;
mod commands;
mod cursor;
mod tui;

use crate::tui::client_ui;
use anyhow::Result;
use clap::{Parser, Subcommand};
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

    #[arg(long, help = "Show only OpenCode usage")]
    opencode: bool,

    #[arg(long, help = "Show only Claude Code usage")]
    claude: bool,

    #[arg(long, help = "Show only Codex CLI usage")]
    codex: bool,

    #[arg(long, help = "Show only Copilot CLI usage")]
    copilot: bool,

    #[arg(long, help = "Show only Gemini CLI usage")]
    gemini: bool,

    #[arg(long, help = "Show only Cursor IDE usage")]
    cursor: bool,

    #[arg(long, help = "Show only Amp usage")]
    amp: bool,

    #[arg(long, help = "Show only Droid usage")]
    droid: bool,

    #[arg(long, help = "Show only OpenClaw usage")]
    openclaw: bool,

    #[arg(long, help = "Show only Hermes Agent usage")]
    hermes: bool,

    #[arg(long, help = "Show only Pi usage")]
    pi: bool,

    #[arg(long, help = "Show only Kimi CLI usage")]
    kimi: bool,

    #[arg(long, help = "Show only Qwen CLI usage")]
    qwen: bool,

    #[arg(long, help = "Show only Roo Code usage")]
    roocode: bool,

    #[arg(long, help = "Show only KiloCode usage")]
    kilocode: bool,

    #[arg(long, help = "Show only Kilo CLI usage")]
    kilo: bool,

    #[arg(long, help = "Show only Mux usage")]
    mux: bool,

    #[arg(long, help = "Show only Crush usage")]
    crush: bool,

    #[arg(long, help = "Show only Synthetic usage")]
    synthetic: bool,

    #[arg(long, help = "Show only today's usage")]
    today: bool,

    #[arg(long, help = "Show last 7 days")]
    week: bool,

    #[arg(long, help = "Show current month")]
    month: bool,

    #[arg(long, help = "Start date (YYYY-MM-DD)")]
    since: Option<String>,

    #[arg(long, help = "End date (YYYY-MM-DD)")]
    until: Option<String>,

    #[arg(long, help = "Filter by year (YYYY)")]
    year: Option<String>,

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
        #[arg(long, help = "Show only OpenCode usage")]
        opencode: bool,
        #[arg(long, help = "Show only Claude Code usage")]
        claude: bool,
        #[arg(long, help = "Show only Codex CLI usage")]
        codex: bool,
        #[arg(long, help = "Show only Copilot CLI usage")]
        copilot: bool,
        #[arg(long, help = "Show only Gemini CLI usage")]
        gemini: bool,
        #[arg(long, help = "Show only Cursor IDE usage")]
        cursor: bool,
        #[arg(long, help = "Show only Amp usage")]
        amp: bool,
        #[arg(long, help = "Show only Droid usage")]
        droid: bool,
        #[arg(long, help = "Show only OpenClaw usage")]
        openclaw: bool,

        #[arg(long, help = "Show only Hermes Agent usage")]
        hermes: bool,
        #[arg(long, help = "Show only Pi usage")]
        pi: bool,
        #[arg(long, help = "Show only Kimi CLI usage")]
        kimi: bool,
        #[arg(long, help = "Show only Qwen CLI usage")]
        qwen: bool,
        #[arg(long, help = "Show only Roo Code usage")]
        roocode: bool,
        #[arg(long, help = "Show only KiloCode usage")]
        kilocode: bool,
        #[arg(long, help = "Show only Kilo CLI usage")]
        kilo: bool,
        #[arg(long, help = "Show only Mux usage")]
        mux: bool,
        #[arg(long, help = "Show only Crush usage")]
        crush: bool,
        #[arg(long, help = "Show only Synthetic usage")]
        synthetic: bool,
        #[arg(long, help = "Show only today's usage")]
        today: bool,
        #[arg(long, help = "Show last 7 days")]
        week: bool,
        #[arg(long, help = "Show current month")]
        month: bool,
        #[arg(long, help = "Start date (YYYY-MM-DD)")]
        since: Option<String>,
        #[arg(long, help = "End date (YYYY-MM-DD)")]
        until: Option<String>,
        #[arg(long, help = "Filter by year (YYYY)")]
        year: Option<String>,
        #[arg(long, help = "Show processing time")]
        benchmark: bool,
        #[arg(
            long,
            value_name = "STRATEGY",
            default_value = "client,model",
            help = "Grouping strategy for --light and --json output: model, client,model, client,provider,model, workspace,model"
        )]
        group_by: String,
        #[arg(long, help = "Disable spinner")]
        no_spinner: bool,
    },
    #[command(about = "Show monthly usage report")]
    Monthly {
        #[arg(long)]
        json: bool,
        #[arg(long)]
        light: bool,
        #[arg(long, help = "Show only OpenCode usage")]
        opencode: bool,
        #[arg(long, help = "Show only Claude Code usage")]
        claude: bool,
        #[arg(long, help = "Show only Codex CLI usage")]
        codex: bool,
        #[arg(long, help = "Show only Copilot CLI usage")]
        copilot: bool,
        #[arg(long, help = "Show only Gemini CLI usage")]
        gemini: bool,
        #[arg(long, help = "Show only Cursor IDE usage")]
        cursor: bool,
        #[arg(long, help = "Show only Amp usage")]
        amp: bool,
        #[arg(long, help = "Show only Droid usage")]
        droid: bool,
        #[arg(long, help = "Show only OpenClaw usage")]
        openclaw: bool,

        #[arg(long, help = "Show only Hermes Agent usage")]
        hermes: bool,
        #[arg(long, help = "Show only Pi usage")]
        pi: bool,
        #[arg(long, help = "Show only Kimi CLI usage")]
        kimi: bool,
        #[arg(long, help = "Show only Qwen CLI usage")]
        qwen: bool,
        #[arg(long, help = "Show only Roo Code usage")]
        roocode: bool,
        #[arg(long, help = "Show only KiloCode usage")]
        kilocode: bool,
        #[arg(long, help = "Show only Kilo CLI usage")]
        kilo: bool,
        #[arg(long, help = "Show only Mux usage")]
        mux: bool,
        #[arg(long, help = "Show only Crush usage")]
        crush: bool,
        #[arg(long, help = "Show only Synthetic usage")]
        synthetic: bool,
        #[arg(long, help = "Show only today's usage")]
        today: bool,
        #[arg(long, help = "Show last 7 days")]
        week: bool,
        #[arg(long, help = "Show current month")]
        month: bool,
        #[arg(long, help = "Start date (YYYY-MM-DD)")]
        since: Option<String>,
        #[arg(long, help = "End date (YYYY-MM-DD)")]
        until: Option<String>,
        #[arg(long, help = "Filter by year (YYYY)")]
        year: Option<String>,
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
        #[arg(long, help = "Show only OpenCode usage")]
        opencode: bool,
        #[arg(long, help = "Show only Claude Code usage")]
        claude: bool,
        #[arg(long, help = "Show only Codex CLI usage")]
        codex: bool,
        #[arg(long, help = "Show only Copilot CLI usage")]
        copilot: bool,
        #[arg(long, help = "Show only Gemini CLI usage")]
        gemini: bool,
        #[arg(long, help = "Show only Cursor IDE usage")]
        cursor: bool,
        #[arg(long, help = "Show only Amp usage")]
        amp: bool,
        #[arg(long, help = "Show only Droid usage")]
        droid: bool,
        #[arg(long, help = "Show only OpenClaw usage")]
        openclaw: bool,
        #[arg(long, help = "Show only Hermes Agent usage")]
        hermes: bool,
        #[arg(long, help = "Show only Pi usage")]
        pi: bool,
        #[arg(long, help = "Show only Kimi CLI usage")]
        kimi: bool,
        #[arg(long, help = "Show only Qwen CLI usage")]
        qwen: bool,
        #[arg(long, help = "Show only Roo Code usage")]
        roocode: bool,
        #[arg(long, help = "Show only KiloCode usage")]
        kilocode: bool,
        #[arg(long, help = "Show only Kilo CLI usage")]
        kilo: bool,
        #[arg(long, help = "Show only Mux usage")]
        mux: bool,
        #[arg(long, help = "Show only Crush usage")]
        crush: bool,
        #[arg(long, help = "Show only Synthetic usage")]
        synthetic: bool,
        #[arg(long, help = "Show only today's usage")]
        today: bool,
        #[arg(long, help = "Show last 7 days")]
        week: bool,
        #[arg(long, help = "Show current month")]
        month: bool,
        #[arg(long, help = "Start date (YYYY-MM-DD)")]
        since: Option<String>,
        #[arg(long, help = "End date (YYYY-MM-DD)")]
        until: Option<String>,
        #[arg(long, help = "Filter by year (YYYY)")]
        year: Option<String>,
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
        #[arg(long, help = "Show only OpenCode usage")]
        opencode: bool,
        #[arg(long, help = "Show only Claude Code usage")]
        claude: bool,
        #[arg(long, help = "Show only Codex CLI usage")]
        codex: bool,
        #[arg(long, help = "Show only Copilot CLI usage")]
        copilot: bool,
        #[arg(long, help = "Show only Gemini CLI usage")]
        gemini: bool,
        #[arg(long, help = "Show only Cursor IDE usage")]
        cursor: bool,
        #[arg(long, help = "Show only Amp usage")]
        amp: bool,
        #[arg(long, help = "Show only Droid usage")]
        droid: bool,
        #[arg(long, help = "Show only OpenClaw usage")]
        openclaw: bool,

        #[arg(long, help = "Show only Hermes Agent usage")]
        hermes: bool,
        #[arg(long, help = "Show only Pi usage")]
        pi: bool,
        #[arg(long, help = "Show only Kimi CLI usage")]
        kimi: bool,
        #[arg(long, help = "Show only Qwen CLI usage")]
        qwen: bool,
        #[arg(long, help = "Show only Roo Code usage")]
        roocode: bool,
        #[arg(long, help = "Show only KiloCode usage")]
        kilocode: bool,
        #[arg(long, help = "Show only Kilo CLI usage")]
        kilo: bool,
        #[arg(long, help = "Show only Mux usage")]
        mux: bool,
        #[arg(long, help = "Show only Crush usage")]
        crush: bool,
        #[arg(long, help = "Show only Synthetic usage")]
        synthetic: bool,
        #[arg(long, help = "Show only today's usage")]
        today: bool,
        #[arg(long, help = "Show last 7 days")]
        week: bool,
        #[arg(long, help = "Show current month")]
        month: bool,
        #[arg(long, help = "Start date (YYYY-MM-DD)")]
        since: Option<String>,
        #[arg(long, help = "End date (YYYY-MM-DD)")]
        until: Option<String>,
        #[arg(long, help = "Filter by year (YYYY)")]
        year: Option<String>,
        #[arg(long, help = "Show processing time")]
        benchmark: bool,
        #[arg(long, help = "Disable spinner")]
        no_spinner: bool,
    },
    #[command(about = "Launch interactive TUI with optional filters")]
    Tui {
        #[arg(long, help = "Show only OpenCode usage")]
        opencode: bool,
        #[arg(long, help = "Show only Claude Code usage")]
        claude: bool,
        #[arg(long, help = "Show only Codex CLI usage")]
        codex: bool,
        #[arg(long, help = "Show only Copilot CLI usage")]
        copilot: bool,
        #[arg(long, help = "Show only Gemini CLI usage")]
        gemini: bool,
        #[arg(long, help = "Show only Cursor IDE usage")]
        cursor: bool,
        #[arg(long, help = "Show only Amp usage")]
        amp: bool,
        #[arg(long, help = "Show only Droid usage")]
        droid: bool,
        #[arg(long, help = "Show only OpenClaw usage")]
        openclaw: bool,

        #[arg(long, help = "Show only Hermes Agent usage")]
        hermes: bool,
        #[arg(long, help = "Show only Pi usage")]
        pi: bool,
        #[arg(long, help = "Show only Kimi CLI usage")]
        kimi: bool,
        #[arg(long, help = "Show only Qwen CLI usage")]
        qwen: bool,
        #[arg(long, help = "Show only Roo Code usage")]
        roocode: bool,
        #[arg(long, help = "Show only KiloCode usage")]
        kilocode: bool,
        #[arg(long, help = "Show only Kilo CLI usage")]
        kilo: bool,
        #[arg(long, help = "Show only Mux usage")]
        mux: bool,
        #[arg(long, help = "Show only Crush usage")]
        crush: bool,
        #[arg(long, help = "Show only Synthetic usage")]
        synthetic: bool,
        #[arg(long, help = "Show only today's usage")]
        today: bool,
        #[arg(long, help = "Show last 7 days")]
        week: bool,
        #[arg(long, help = "Show current month")]
        month: bool,
        #[arg(long, help = "Start date (YYYY-MM-DD)")]
        since: Option<String>,
        #[arg(long, help = "End date (YYYY-MM-DD)")]
        until: Option<String>,
        #[arg(long, help = "Filter by year (YYYY)")]
        year: Option<String>,
    },
    #[command(about = "Submit usage data to the Tokscale social platform")]
    Submit {
        #[arg(long, help = "Include only OpenCode data")]
        opencode: bool,
        #[arg(long, help = "Include only Claude Code data")]
        claude: bool,
        #[arg(long, help = "Include only Codex CLI data")]
        codex: bool,
        #[arg(long, help = "Include only Copilot CLI data")]
        copilot: bool,
        #[arg(long, help = "Include only Gemini CLI data")]
        gemini: bool,
        #[arg(long, help = "Include only Cursor IDE data")]
        cursor: bool,
        #[arg(long, help = "Include only Amp data")]
        amp: bool,
        #[arg(long, help = "Include only Droid data")]
        droid: bool,
        #[arg(long, help = "Include only OpenClaw data")]
        openclaw: bool,
        #[arg(long, help = "Include only Hermes Agent data")]
        hermes: bool,
        #[arg(long, help = "Include only Pi data")]
        pi: bool,
        #[arg(long, help = "Include only Kimi CLI data")]
        kimi: bool,
        #[arg(long, help = "Include only Qwen CLI data")]
        qwen: bool,
        #[arg(long, help = "Show only Roo Code usage")]
        roocode: bool,
        #[arg(long, help = "Show only KiloCode usage")]
        kilocode: bool,
        #[arg(long, help = "Show only Kilo CLI usage")]
        kilo: bool,
        #[arg(long, help = "Show only Mux usage")]
        mux: bool,
        #[arg(long, help = "Show only Crush usage")]
        crush: bool,
        #[arg(long, help = "Show only Synthetic usage")]
        synthetic: bool,
        #[arg(long, help = "Submit only today's usage")]
        today: bool,
        #[arg(long, help = "Submit last 7 days")]
        week: bool,
        #[arg(long, help = "Submit current month")]
        month: bool,
        #[arg(long, help = "Start date (YYYY-MM-DD)")]
        since: Option<String>,
        #[arg(long, help = "End date (YYYY-MM-DD)")]
        until: Option<String>,
        #[arg(long, help = "Filter by year (YYYY)")]
        year: Option<String>,
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
        #[arg(long, help = "Show only OpenCode usage")]
        opencode: bool,
        #[arg(long, help = "Show only Claude Code usage")]
        claude: bool,
        #[arg(long, help = "Show only Codex CLI usage")]
        codex: bool,
        #[arg(long, help = "Show only Copilot CLI usage")]
        copilot: bool,
        #[arg(long, help = "Show only Gemini CLI usage")]
        gemini: bool,
        #[arg(long, help = "Show only Cursor IDE usage")]
        cursor: bool,
        #[arg(long, help = "Show only Amp usage")]
        amp: bool,
        #[arg(long, help = "Show only Droid usage")]
        droid: bool,
        #[arg(long, help = "Show only OpenClaw usage")]
        openclaw: bool,

        #[arg(long, help = "Show only Hermes Agent usage")]
        hermes: bool,
        #[arg(long, help = "Show only Pi usage")]
        pi: bool,
        #[arg(long, help = "Show only Kimi CLI usage")]
        kimi: bool,
        #[arg(long, help = "Show only Qwen CLI usage")]
        qwen: bool,
        #[arg(long, help = "Show only Roo Code usage")]
        roocode: bool,
        #[arg(long, help = "Show only KiloCode usage")]
        kilocode: bool,
        #[arg(long, help = "Show only Kilo CLI usage")]
        kilo: bool,
        #[arg(long, help = "Show only Mux usage")]
        mux: bool,
        #[arg(long, help = "Show only Crush usage")]
        crush: bool,
        #[arg(long, help = "Show only Synthetic usage")]
        synthetic: bool,
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
    #[command(about = "Delete all submitted usage data from the server")]
    DeleteSubmittedData,
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
    #[command(about = "Switch active Cursor account")]
    Switch {
        #[arg(help = "Account label or id")]
        name: String,
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
            opencode,
            claude,
            codex,
            copilot,
            gemini,
            cursor,
            amp,
            droid,
            openclaw,
            hermes,
            pi,
            kimi,
            qwen,
            roocode,
            kilocode,
            kilo,
            mux,
            crush,
            synthetic,
            today,
            week,
            month,
            since,
            until,
            year,
            benchmark,
            group_by,
            no_spinner,
        }) => {
            use tokscale_core::GroupBy;

            let group_by: GroupBy = group_by.parse().unwrap_or_else(|e| {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            });
            let clients = build_client_filter(ClientFlags {
                opencode,
                claude,
                codex,
                copilot,
                gemini,
                cursor,
                amp,
                droid,
                openclaw,
                hermes,
                pi,
                kimi,
                qwen,
                roocode,
                kilocode,
                kilo,
                mux,
                crush,
                synthetic,
            });
            let (since, until) = build_date_filter(today, week, month, since, until);
            let year = normalize_year_filter(today, week, month, year);
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
                )
            } else {
                ensure_home_supported_for_tui(&cli.home)?;
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
            opencode,
            claude,
            codex,
            copilot,
            gemini,
            cursor,
            amp,
            droid,
            openclaw,
            hermes,
            pi,
            kimi,
            qwen,
            roocode,
            kilocode,
            kilo,
            mux,
            crush,
            synthetic,
            today,
            week,
            month,
            since,
            until,
            year,
            benchmark,
            no_spinner,
        }) => {
            let clients = build_client_filter(ClientFlags {
                opencode,
                claude,
                codex,
                copilot,
                gemini,
                cursor,
                amp,
                droid,
                openclaw,
                hermes,
                pi,
                kimi,
                qwen,
                roocode,
                kilocode,
                kilo,
                mux,
                crush,
                synthetic,
            });
            let (since, until) = build_date_filter(today, week, month, since, until);
            let year = normalize_year_filter(today, week, month, year);
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
            light: _,
            opencode,
            claude,
            codex,
            copilot,
            gemini,
            cursor,
            amp,
            droid,
            openclaw,
            hermes,
            pi,
            kimi,
            qwen,
            roocode,
            kilocode,
            kilo,
            mux,
            crush,
            synthetic,
            today,
            week,
            month,
            since,
            until,
            year,
            benchmark,
            no_spinner,
        }) => {
            let clients = build_client_filter(ClientFlags {
                opencode,
                claude,
                codex,
                copilot,
                gemini,
                cursor,
                amp,
                droid,
                openclaw,
                hermes,
                pi,
                kimi,
                qwen,
                roocode,
                kilocode,
                kilo,
                mux,
                crush,
                synthetic,
            });
            let (since, until) = build_date_filter(today, week, month, since, until);
            let year = normalize_year_filter(today, week, month, year);
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
            opencode,
            claude,
            codex,
            copilot,
            gemini,
            cursor,
            amp,
            droid,
            openclaw,
            hermes,
            pi,
            kimi,
            qwen,
            roocode,
            kilocode,
            kilo,
            mux,
            crush,
            synthetic,
            today,
            week,
            month,
            since,
            until,
            year,
            benchmark,
            no_spinner,
        }) => {
            let clients = build_client_filter(ClientFlags {
                opencode,
                claude,
                codex,
                copilot,
                gemini,
                cursor,
                amp,
                droid,
                openclaw,
                hermes,
                pi,
                kimi,
                qwen,
                roocode,
                kilocode,
                kilo,
                mux,
                crush,
                synthetic,
            });
            let (since, until) = build_date_filter(today, week, month, since, until);
            let year = normalize_year_filter(today, week, month, year);
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
        Some(Commands::Tui {
            opencode,
            claude,
            codex,
            copilot,
            gemini,
            cursor,
            amp,
            droid,
            openclaw,
            hermes,
            pi,
            kimi,
            qwen,
            roocode,
            kilocode,
            kilo,
            mux,
            crush,
            synthetic,
            today,
            week,
            month,
            since,
            until,
            year,
        }) => {
            ensure_home_supported_for_tui(&cli.home)?;
            let clients = build_client_filter(ClientFlags {
                opencode,
                claude,
                codex,
                copilot,
                gemini,
                cursor,
                amp,
                droid,
                openclaw,
                hermes,
                pi,
                kimi,
                qwen,
                roocode,
                kilocode,
                kilo,
                mux,
                crush,
                synthetic,
            });
            let (since, until) = build_date_filter(today, week, month, since, until);
            let year = normalize_year_filter(today, week, month, year);
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
            opencode,
            claude,
            codex,
            copilot,
            gemini,
            cursor,
            amp,
            droid,
            openclaw,
            hermes,
            pi,
            kimi,
            qwen,
            roocode,
            kilocode,
            kilo,
            mux,
            crush,
            synthetic,
            today,
            week,
            month,
            since,
            until,
            year,
            dry_run,
        }) => {
            reject_unsupported_home_override(&cli.home, "submit")?;
            let clients = build_client_filter(ClientFlags {
                opencode,
                claude,
                codex,
                copilot,
                gemini,
                cursor,
                amp,
                droid,
                openclaw,
                hermes,
                pi,
                kimi,
                qwen,
                roocode,
                kilocode,
                kilo,
                mux,
                crush,
                synthetic,
            });
            let (since, until) = build_date_filter(today, week, month, since, until);
            let year = normalize_year_filter(today, week, month, year);
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
            opencode,
            claude,
            codex,
            copilot,
            gemini,
            cursor,
            amp,
            droid,
            openclaw,
            hermes,
            pi,
            kimi,
            qwen,
            roocode,
            kilocode,
            kilo,
            mux,
            crush,
            synthetic,
            short,
            agents,
            clients,
            disable_pinned,
            no_spinner: _,
        }) => {
            reject_unsupported_home_override(&cli.home, "wrapped")?;
            let client_filter = build_client_filter(ClientFlags {
                opencode,
                claude,
                codex,
                copilot,
                gemini,
                cursor,
                amp,
                droid,
                openclaw,
                hermes,
                pi,
                kimi,
                qwen,
                roocode,
                kilocode,
                kilo,
                mux,
                crush,
                synthetic,
            });
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
        Some(Commands::DeleteSubmittedData) => {
            reject_unsupported_home_override(&cli.home, "delete-submitted-data")?;
            run_delete_data_command()
        }
        None => {
            let clients = build_client_filter(ClientFlags {
                opencode: cli.opencode,
                claude: cli.claude,
                codex: cli.codex,
                copilot: cli.copilot,
                gemini: cli.gemini,
                cursor: cli.cursor,
                amp: cli.amp,
                droid: cli.droid,
                openclaw: cli.openclaw,
                hermes: cli.hermes,
                pi: cli.pi,
                kimi: cli.kimi,
                qwen: cli.qwen,
                roocode: cli.roocode,
                kilocode: cli.kilocode,
                kilo: cli.kilo,
                mux: cli.mux,
                crush: cli.crush,
                synthetic: cli.synthetic,
            });
            let (since, until) =
                build_date_filter(cli.today, cli.week, cli.month, cli.since, cli.until);
            let year = normalize_year_filter(cli.today, cli.week, cli.month, cli.year);
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
                    cli.today,
                    cli.week,
                    cli.month,
                    group_by,
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
                    cli.today,
                    cli.week,
                    cli.month,
                    group_by,
                )
            } else {
                ensure_home_supported_for_tui(&cli.home)?;
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

struct ClientFlags {
    opencode: bool,
    claude: bool,
    codex: bool,
    copilot: bool,
    gemini: bool,
    cursor: bool,
    amp: bool,
    droid: bool,
    openclaw: bool,
    hermes: bool,
    pi: bool,
    kimi: bool,
    qwen: bool,
    roocode: bool,
    kilocode: bool,
    kilo: bool,
    mux: bool,
    crush: bool,
    synthetic: bool,
}

fn build_client_filter(flags: ClientFlags) -> Option<Vec<String>> {
    use tokscale_core::ClientId;

    let mut clients: Vec<String> = [
        (ClientId::OpenCode, flags.opencode),
        (ClientId::Claude, flags.claude),
        (ClientId::Codex, flags.codex),
        (ClientId::Copilot, flags.copilot),
        (ClientId::Gemini, flags.gemini),
        (ClientId::Cursor, flags.cursor),
        (ClientId::Amp, flags.amp),
        (ClientId::Droid, flags.droid),
        (ClientId::OpenClaw, flags.openclaw),
        (ClientId::Hermes, flags.hermes),
        (ClientId::Pi, flags.pi),
        (ClientId::Kimi, flags.kimi),
        (ClientId::Qwen, flags.qwen),
        (ClientId::RooCode, flags.roocode),
        (ClientId::KiloCode, flags.kilocode),
        (ClientId::Kilo, flags.kilo),
        (ClientId::Mux, flags.mux),
        (ClientId::Crush, flags.crush),
    ]
    .into_iter()
    .filter(|(_, enabled)| *enabled)
    .map(|(client, _)| client.as_str().to_string())
    .collect();

    if flags.synthetic {
        clients.push("synthetic".to_string());
    }

    if clients.is_empty() {
        None
    } else {
        Some(clients)
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
) -> Result<()> {
    use std::time::Instant;
    use tokio::runtime::Runtime;
    use tokscale_core::{get_model_report, GroupBy, ReportOptions};

    let date_range = get_date_range_label(today, week, month_flag, &since, &until, &year);

    let spinner = if no_spinner {
        None
    } else {
        Some(LightSpinner::start("Scanning session data..."))
    };
    let use_env_roots = use_env_roots(&home_dir);
    let start = Instant::now();
    let rt = Runtime::new()?;
    let report = rt
        .block_on(async {
            get_model_report(ReportOptions {
                home_dir,
                use_env_roots,
                clients,
                since,
                until,
                year,
                group_by: group_by.clone(),
                scanner_settings: tui::settings::load_scanner_settings(),
            })
            .await
        })
        .map_err(|e| anyhow::anyhow!(e))?;

    if let Some(spinner) = spinner {
        spinner.stop();
    }

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

    let spinner = if no_spinner {
        None
    } else {
        Some(LightSpinner::start("Scanning session data..."))
    };
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

    let spinner = if no_spinner {
        None
    } else {
        Some(LightSpinner::start("Scanning session data..."))
    };
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
    if total_tokens == 0 {
        "—".to_string()
    } else {
        let cost_per_m = cost * 1_000_000.0 / total_tokens as f64;
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
        "droid" => "Droid".to_string(),
        "crush" => "Crush".to_string(),
        "openclaw" => "openclaw".to_string(),
        "hermes" => "Hermes Agent".to_string(),
        "pi" => "Pi".to_string(),
        other => other.to_string(),
    }
}

fn run_clients_command(json: bool) -> Result<()> {
    use tokscale_core::{parse_local_clients, ClientId, LocalParseOptions};

    let home_dir =
        dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?;

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
        scanner_settings: tui::settings::load_scanner_settings(),
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
    }

    // Collect extra dirs from TOKSCALE_EXTRA_DIRS for display (reuse core parser)
    let extra_dirs_val = std::env::var("TOKSCALE_EXTRA_DIRS").unwrap_or_default();
    let all_clients: std::collections::HashSet<ClientId> = ClientId::iter().collect();
    let extra_dirs: Vec<(ClientId, String)> =
        tokscale_core::parse_extra_dirs(&extra_dirs_val, &all_clients);
    let copilot_exporter_path = tokscale_core::copilot_exporter_path();

    let clients: Vec<ClientRow> = ClientId::iter()
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

            let extra_paths: Vec<ExtraPath> = extra_dirs
                .iter()
                .filter(|(c, _)| *c == client)
                .map(|(_, path)| ExtraPath {
                    path: path.clone(),
                    exists: Path::new(path).exists(),
                })
                .collect();
            let exporter_status = (client == ClientId::Copilot && copilot_exporter_path.is_some())
                .then(|| "configured".to_string());

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
                exporter_status,
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
                let extra_desc: Vec<String> = row
                    .extra_paths
                    .iter()
                    .map(|ep| describe_path(&ep.path, ep.exists))
                    .collect();
                println!(
                    "  {}",
                    format!("extra: {}", extra_desc.join(", ")).bright_black()
                );
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
    dirs::config_dir().map(|d| d.join("tokscale").join("star-cache.json"))
}

fn load_star_cache(username: &str) -> Option<StarCache> {
    let path = star_cache_path()?;
    let content = std::fs::read_to_string(path).ok()?;
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
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&path, content);
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
    use tokscale_core::{generate_graph, ClientId, GroupBy, ReportOptions};

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
    // We load with all clients and no date filters (default TUI view)
    // to maximize cache hit rate.
    {
        use crate::tui::{save_cached_data, DataLoader};
        use std::collections::HashSet;

        let all_clients: Vec<ClientId> = ClientId::iter().collect();
        let enabled_set: HashSet<ClientId> = all_clients.iter().copied().collect();
        let loader = DataLoader::with_filters(None, None, None, None);
        if let Ok(data) = loader.load(&all_clients, &GroupBy::default(), false) {
            save_cached_data(&data, &enabled_set, false, &GroupBy::default());
        }
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
        CursorSubcommand::Switch { name } => cursor::run_cursor_switch(&name),
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

    #[test]
    fn test_build_client_filter_all_false() {
        let flags = ClientFlags {
            opencode: false,
            claude: false,
            codex: false,
            copilot: false,
            gemini: false,
            cursor: false,
            amp: false,
            droid: false,
            openclaw: false,
            hermes: false,
            pi: false,
            kimi: false,
            qwen: false,
            roocode: false,
            kilocode: false,
            kilo: false,
            mux: false,
            crush: false,
            synthetic: false,
        };
        assert_eq!(build_client_filter(flags), None);
    }

    #[test]
    fn test_build_client_filter_single_client() {
        let flags = ClientFlags {
            opencode: true,
            claude: false,
            codex: false,
            copilot: false,
            gemini: false,
            cursor: false,
            amp: false,
            droid: false,
            openclaw: false,
            hermes: false,
            pi: false,
            kimi: false,
            qwen: false,
            roocode: false,
            kilocode: false,
            kilo: false,
            mux: false,
            crush: false,
            synthetic: false,
        };
        assert_eq!(
            build_client_filter(flags),
            Some(vec!["opencode".to_string()])
        );
    }

    #[test]
    fn test_build_client_filter_multiple_clients() {
        let flags = ClientFlags {
            opencode: true,
            claude: true,
            codex: false,
            copilot: false,
            gemini: false,
            cursor: false,
            amp: false,
            droid: false,
            openclaw: false,
            hermes: false,
            pi: true,
            kimi: false,
            qwen: false,
            roocode: false,
            kilocode: false,
            kilo: false,
            mux: false,
            crush: false,
            synthetic: false,
        };
        assert_eq!(
            build_client_filter(flags),
            Some(vec![
                "opencode".to_string(),
                "claude".to_string(),
                "pi".to_string()
            ])
        );
    }

    #[test]
    fn test_build_client_filter_synthetic_only() {
        let flags = ClientFlags {
            opencode: false,
            claude: false,
            codex: false,
            copilot: false,
            gemini: false,
            cursor: false,
            amp: false,
            droid: false,
            openclaw: false,
            hermes: false,
            pi: false,
            kimi: false,
            qwen: false,
            roocode: false,
            kilocode: false,
            kilo: false,
            mux: false,
            crush: false,
            synthetic: true,
        };
        assert_eq!(
            build_client_filter(flags),
            Some(vec!["synthetic".to_string()])
        );
    }

    #[test]
    fn test_build_client_filter_all_clients() {
        let flags = ClientFlags {
            opencode: true,
            claude: true,
            codex: true,
            copilot: true,
            gemini: true,
            cursor: true,
            amp: true,
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
            synthetic: true,
        };
        let result = build_client_filter(flags);
        assert!(result.is_some());
        let sources = result.unwrap();
        let expected_len = tokscale_core::ClientId::iter().count() + 1; // synthetic is not in ClientId
        assert_eq!(sources.len(), expected_len);
        assert!(sources.contains(&"opencode".to_string()));
        assert!(sources.contains(&"claude".to_string()));
        assert!(sources.contains(&"codex".to_string()));
        assert!(sources.contains(&"copilot".to_string()));
        assert!(sources.contains(&"gemini".to_string()));
        assert!(sources.contains(&"cursor".to_string()));
        assert!(sources.contains(&"amp".to_string()));
        assert!(sources.contains(&"droid".to_string()));
        assert!(sources.contains(&"openclaw".to_string()));
        assert!(sources.contains(&"hermes".to_string()));
        assert!(sources.contains(&"pi".to_string()));
        assert!(sources.contains(&"kimi".to_string()));
        assert!(sources.contains(&"qwen".to_string()));
        assert!(sources.contains(&"roocode".to_string()));
        assert!(sources.contains(&"kilocode".to_string()));
        assert!(sources.contains(&"kilo".to_string()));
        assert!(sources.contains(&"mux".to_string()));
        assert!(sources.contains(&"crush".to_string()));
        assert!(sources.contains(&"synthetic".to_string()));
    }

    #[test]
    fn test_default_submit_clients_excludes_crush() {
        let clients = default_submit_clients();
        assert!(clients.contains(&"synthetic".to_string()));
        assert!(!clients.contains(&"crush".to_string()));
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
        assert_eq!(forward, true);
    }

    #[test]
    fn test_light_spinner_scanner_state_forward_mid() {
        let (position, forward) = LightSpinner::scanner_state(4);
        assert_eq!(position, 4);
        assert_eq!(forward, true);
    }

    #[test]
    fn test_light_spinner_scanner_state_forward_end() {
        let (position, forward) = LightSpinner::scanner_state(7);
        assert_eq!(position, 7);
        assert_eq!(forward, true);
    }

    #[test]
    fn test_light_spinner_scanner_state_hold_end() {
        let (position, forward) = LightSpinner::scanner_state(8);
        assert_eq!(position, 7);
        assert_eq!(forward, true);
    }

    #[test]
    fn test_light_spinner_scanner_state_backward_start() {
        let (position, forward) = LightSpinner::scanner_state(17);
        assert_eq!(position, 6);
        assert_eq!(forward, false);
    }

    #[test]
    fn test_light_spinner_scanner_state_backward_end() {
        let (position, forward) = LightSpinner::scanner_state(23);
        assert_eq!(position, 0);
        assert_eq!(forward, false);
    }

    #[test]
    fn test_light_spinner_scanner_state_hold_start() {
        let (position, forward) = LightSpinner::scanner_state(24);
        assert_eq!(position, 0);
        assert_eq!(forward, false);
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
}
