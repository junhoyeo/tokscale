#!/usr/bin/env node
/**
 * Tokscale CLI
 * Display OpenCode, Claude Code, Codex, Gemini, and Cursor usage with dynamic width tables
 * 
 * All heavy computation is done in the native Rust module.
 */

import { Command } from "commander";
import pc from "picocolors";
import { login, logout, whoami } from "./auth.js";
import { submit } from "./submit.js";
import { PricingFetcher } from "./pricing.js";
import {
  loadCursorCredentials,
  saveCursorCredentials,
  clearCursorCredentials,
  validateCursorSession,
  readCursorUsage,
  getCursorCredentialsPath,
  syncCursorCache,
} from "./cursor.js";
import {
  createUsageTable,
  formatUsageRow,
  formatTotalsRow,
  formatNumber,
  formatCurrency,
  formatModelName,
} from "./table.js";
import {
  isNativeAvailable,
  getNativeVersion,
  parseLocalSourcesAsync,
  finalizeReportAsync,
  finalizeMonthlyReportAsync,
  finalizeGraphAsync,
  type ModelReport,
  type MonthlyReport,
  type ParsedMessages,
} from "./native.js";
import { createSpinner } from "./spinner.js";
import * as fs from "node:fs";
import { performance } from "node:perf_hooks";
import type { SourceType } from "./graph-types.js";
import type { TUIOptions, TabType } from "./tui/types/index.js";

interface FilterOptions {
  opencode?: boolean;
  claude?: boolean;
  codex?: boolean;
  gemini?: boolean;
  cursor?: boolean;
}

interface DateFilterOptions {
  since?: string;
  until?: string;
  year?: string;
  today?: boolean;
  week?: boolean;
  month?: boolean;
}

interface CursorSyncResult {
  /** Whether a sync was attempted (true if credentials exist) */
  attempted: boolean;
  /** Whether the sync succeeded */
  synced: boolean;
  /** Number of usage events fetched */
  rows: number;
  /** Error message if sync failed */
  error?: string;
}

// =============================================================================
// Date Helpers
// =============================================================================

function formatDate(date: Date): string {
  return date.toISOString().split("T")[0];
}

function getDateFilters(options: DateFilterOptions): { since?: string; until?: string; year?: string } {
  const today = new Date();
  
  // --today: just today
  if (options.today) {
    const todayStr = formatDate(today);
    return { since: todayStr, until: todayStr };
  }
  
  // --week: last 7 days
  if (options.week) {
    const weekAgo = new Date(today);
    weekAgo.setDate(weekAgo.getDate() - 6); // Include today = 7 days
    return { since: formatDate(weekAgo), until: formatDate(today) };
  }
  
  // --month: current calendar month
  if (options.month) {
    const startOfMonth = new Date(today.getFullYear(), today.getMonth(), 1);
    return { since: formatDate(startOfMonth), until: formatDate(today) };
  }
  
  // Explicit filters
  return {
    since: options.since,
    until: options.until,
    year: options.year,
  };
}

function getDateRangeLabel(options: DateFilterOptions): string | null {
  if (options.today) return "Today";
  if (options.week) return "Last 7 days";
  if (options.month) {
    const today = new Date();
    return today.toLocaleString("en-US", { month: "long", year: "numeric" } as Intl.DateTimeFormatOptions);
  }
  if (options.year) return options.year;
  if (options.since || options.until) {
    const parts: string[] = [];
    if (options.since) parts.push(`from ${options.since}`);
    if (options.until) parts.push(`to ${options.until}`);
    return parts.join(" ");
  }
  return null;
}

function buildTUIOptions(
  options: FilterOptions & DateFilterOptions,
  initialTab?: TabType
): TUIOptions {
  const dateFilters = getDateFilters(options);
  const enabledSources = getEnabledSources(options);

  return {
    initialTab,
    enabledSources: enabledSources as TUIOptions["enabledSources"],
    since: dateFilters.since,
    until: dateFilters.until,
    year: dateFilters.year,
  };
}

async function main() {
  const program = new Command();

  program
    .name("tokscale")
    .description("Token Usage Leaderboard CLI - Track AI coding costs across OpenCode, Claude Code, Codex, Gemini, and Cursor")
    .version("1.0.2");

  program
    .command("monthly")
    .description("Show monthly usage report (launches TUI by default)")
    .option("--light", "Use legacy CLI table output instead of TUI")
    .option("--json", "Output as JSON (for scripting)")
    .option("--opencode", "Show only OpenCode usage")
    .option("--claude", "Show only Claude Code usage")
    .option("--codex", "Show only Codex CLI usage")
    .option("--gemini", "Show only Gemini CLI usage")
    .option("--cursor", "Show only Cursor IDE usage")
    .option("--today", "Show only today's usage")
    .option("--week", "Show last 7 days")
    .option("--month", "Show current month")
    .option("--since <date>", "Start date (YYYY-MM-DD)")
    .option("--until <date>", "End date (YYYY-MM-DD)")
    .option("--year <year>", "Filter to specific year")
    .option("--benchmark", "Show processing time")
    .action(async (options) => {
      if (options.json) {
        await outputJsonReport("monthly", options);
      } else if (options.light) {
        await showMonthlyReport(options);
      } else {
        const { launchTUI } = await import("./tui/index.js");
        await launchTUI(buildTUIOptions(options, "daily"));
      }
    });

  program
    .command("models")
    .description("Show usage breakdown by model (launches TUI by default)")
    .option("--light", "Use legacy CLI table output instead of TUI")
    .option("--json", "Output as JSON (for scripting)")
    .option("--opencode", "Show only OpenCode usage")
    .option("--claude", "Show only Claude Code usage")
    .option("--codex", "Show only Codex CLI usage")
    .option("--gemini", "Show only Gemini CLI usage")
    .option("--cursor", "Show only Cursor IDE usage")
    .option("--today", "Show only today's usage")
    .option("--week", "Show last 7 days")
    .option("--month", "Show current month")
    .option("--since <date>", "Start date (YYYY-MM-DD)")
    .option("--until <date>", "End date (YYYY-MM-DD)")
    .option("--year <year>", "Filter to specific year")
    .option("--benchmark", "Show processing time")
    .action(async (options) => {
      if (options.json) {
        await outputJsonReport("models", options);
      } else if (options.light) {
        await showModelReport(options);
      } else {
        const { launchTUI } = await import("./tui/index.js");
        await launchTUI(buildTUIOptions(options, "model"));
      }
    });

  program
    .command("graph")
    .description("Export contribution graph data as JSON")
    .option("--output <file>", "Write to file instead of stdout")
    .option("--opencode", "Include only OpenCode data")
    .option("--claude", "Include only Claude Code data")
    .option("--codex", "Include only Codex CLI data")
    .option("--gemini", "Include only Gemini CLI data")
    .option("--cursor", "Include only Cursor IDE data")
    .option("--today", "Show only today's usage")
    .option("--week", "Show last 7 days")
    .option("--month", "Show current month")
    .option("--since <date>", "Start date (YYYY-MM-DD)")
    .option("--until <date>", "End date (YYYY-MM-DD)")
    .option("--year <year>", "Filter to specific year")
    .option("--benchmark", "Show processing time")
    .action(async (options) => {
      await handleGraphCommand(options);
    });

  // =========================================================================
  // Authentication Commands
  // =========================================================================

  program
    .command("login")
    .description("Login to Tokscale (opens browser for GitHub auth)")
    .action(async () => {
      await login();
    });

  program
    .command("logout")
    .description("Logout from Tokscale")
    .action(async () => {
      await logout();
    });

  program
    .command("whoami")
    .description("Show current logged in user")
    .action(async () => {
      await whoami();
    });

  // =========================================================================
  // Submit Command
  // =========================================================================

  program
    .command("submit")
    .description("Submit your usage data to Tokscale")
    .option("--opencode", "Include only OpenCode data")
    .option("--claude", "Include only Claude Code data")
    .option("--codex", "Include only Codex CLI data")
    .option("--gemini", "Include only Gemini CLI data")
    .option("--cursor", "Include only Cursor IDE data")
    .option("--since <date>", "Start date (YYYY-MM-DD)")
    .option("--until <date>", "End date (YYYY-MM-DD)")
    .option("--year <year>", "Filter to specific year")
    .option("--dry-run", "Show what would be submitted without actually submitting")
    .action(async (options) => {
      await submit({
        opencode: options.opencode,
        claude: options.claude,
        codex: options.codex,
        gemini: options.gemini,
        cursor: options.cursor,
        since: options.since,
        until: options.until,
        year: options.year,
        dryRun: options.dryRun,
      });
    });

  // =========================================================================
  // Interactive TUI Command
  // =========================================================================

  program
    .command("tui")
    .description("Launch interactive terminal UI")
    .option("--opencode", "Show only OpenCode usage")
    .option("--claude", "Show only Claude Code usage")
    .option("--codex", "Show only Codex CLI usage")
    .option("--gemini", "Show only Gemini CLI usage")
    .option("--cursor", "Show only Cursor IDE usage")
    .option("--today", "Show only today's usage")
    .option("--week", "Show last 7 days")
    .option("--month", "Show current month")
    .option("--since <date>", "Start date (YYYY-MM-DD)")
    .option("--until <date>", "End date (YYYY-MM-DD)")
    .option("--year <year>", "Filter to specific year")
    .action(async (options) => {
      const { launchTUI } = await import("./tui/index.js");
      await launchTUI(buildTUIOptions(options));
    });

  // =========================================================================
  // Cursor IDE Authentication Commands
  // =========================================================================

  const cursorCommand = program
    .command("cursor")
    .description("Cursor IDE integration commands");

  cursorCommand
    .command("login")
    .description("Login to Cursor (paste your session token)")
    .action(async () => {
      await cursorLogin();
    });

  cursorCommand
    .command("logout")
    .description("Logout from Cursor")
    .action(async () => {
      await cursorLogout();
    });

  cursorCommand
    .command("status")
    .description("Check Cursor authentication status")
    .action(async () => {
      await cursorStatus();
    });

  // Check if a subcommand was provided
  const args = process.argv.slice(2);
  const firstArg = args[0] || '';
  // Global flags should go to main program
  const isGlobalFlag = ['--help', '-h', '--version', '-V'].includes(firstArg);
  const hasSubcommand = args.length > 0 && !firstArg.startsWith('-');
  const knownCommands = ['monthly', 'models', 'graph', 'login', 'logout', 'whoami', 'submit', 'cursor', 'tui', 'help'];
  const isKnownCommand = hasSubcommand && knownCommands.includes(firstArg);

  if (isKnownCommand || isGlobalFlag) {
    // Run the specified subcommand or show full help/version
    await program.parseAsync();
  } else {
    // No subcommand - launch TUI by default, or legacy CLI with --light, or JSON with --json
    const defaultProgram = new Command();
    defaultProgram
      .option("--light", "Use legacy CLI table output instead of TUI")
      .option("--json", "Output as JSON (for scripting)")
      .option("--opencode", "Show only OpenCode usage")
      .option("--claude", "Show only Claude Code usage")
      .option("--codex", "Show only Codex CLI usage")
      .option("--gemini", "Show only Gemini CLI usage")
      .option("--cursor", "Show only Cursor IDE usage")
      .option("--today", "Show only today's usage")
      .option("--week", "Show last 7 days")
      .option("--month", "Show current month")
      .option("--since <date>", "Start date (YYYY-MM-DD)")
      .option("--until <date>", "End date (YYYY-MM-DD)")
      .option("--year <year>", "Filter to specific year")
      .option("--benchmark", "Show processing time")
      .parse();
    
    const opts = defaultProgram.opts();
    if (opts.json) {
      await outputJsonReport("models", opts);
    } else if (opts.light) {
      await showModelReport(opts);
    } else {
      const { launchTUI } = await import("./tui/index.js");
      await launchTUI(buildTUIOptions(opts));
    }
  }
}

function getEnabledSources(options: FilterOptions): SourceType[] | undefined {
  const hasFilter = options.opencode || options.claude || options.codex || options.gemini || options.cursor;
  if (!hasFilter) return undefined; // All sources

  const sources: SourceType[] = [];
  if (options.opencode) sources.push("opencode");
  if (options.claude) sources.push("claude");
  if (options.codex) sources.push("codex");
  if (options.gemini) sources.push("gemini");
  if (options.cursor) sources.push("cursor");
  return sources;
}

async function ensureNativeModule(): Promise<void> {
  if (!isNativeAvailable()) {
    console.error(pc.red("Error: Native Rust module not available."));
    console.error(pc.gray("Run 'yarn build:core' to build the native module."));
    process.exit(1);
  }
}

async function fetchPricingData(): Promise<PricingFetcher> {
  const fetcher = new PricingFetcher();
  await fetcher.fetchPricing();
  return fetcher;
}

/**
 * Sync Cursor usage data from API to local cache.
 * Only attempts sync if user is authenticated with Cursor.
 */
async function syncCursorData(): Promise<CursorSyncResult> {
  const credentials = loadCursorCredentials();
  if (!credentials) {
    return { attempted: false, synced: false, rows: 0 };
  }

  const result = await syncCursorCache();
  return {
    attempted: true,
    synced: result.synced,
    rows: result.rows,
    error: result.error,
  };
}

interface LoadedDataSources {
  fetcher: PricingFetcher;
  cursorSync: CursorSyncResult;
  localMessages: ParsedMessages | null;
}

/**
 * Load all data sources in parallel (two-phase optimization):
 * - Cursor API sync (network)
 * - Pricing fetch (network)
 * - Local file parsing (CPU/IO) - OpenCode, Claude, Codex, Gemini
 * 
 * This overlaps network I/O with local file parsing for better performance.
 */
async function loadDataSourcesParallel(
  localSources: SourceType[],
  dateFilters: { since?: string; until?: string; year?: string }
): Promise<LoadedDataSources> {
  // Skip local parsing if no local sources requested (e.g., cursor-only mode)
  const shouldParseLocal = localSources.length > 0;

  // Use Promise.allSettled for graceful degradation
  const [cursorResult, pricingResult, localResult] = await Promise.allSettled([
    syncCursorData(),
    fetchPricingData(),
    // Parse local sources in parallel (excludes Cursor) - skip if empty
    shouldParseLocal
      ? parseLocalSourcesAsync({
          sources: localSources.filter(s => s !== 'cursor'),
          since: dateFilters.since,
          until: dateFilters.until,
          year: dateFilters.year,
        })
      : Promise.resolve(null),
  ]);

  // Handle partial failures gracefully
  const cursorSync: CursorSyncResult = cursorResult.status === 'fulfilled'
    ? cursorResult.value
    : { attempted: true, synced: false, rows: 0, error: 'Cursor sync failed' };

  const fetcher: PricingFetcher = pricingResult.status === 'fulfilled'
    ? pricingResult.value
    : new PricingFetcher(); // Empty pricing → costs = 0

  const localMessages: ParsedMessages | null = localResult.status === 'fulfilled'
    ? localResult.value
    : null;

  return { fetcher, cursorSync, localMessages };
}

async function showModelReport(options: FilterOptions & DateFilterOptions & { benchmark?: boolean }) {
  await ensureNativeModule();

  const dateFilters = getDateFilters(options);
  const enabledSources = getEnabledSources(options);
  const onlyCursor = enabledSources?.length === 1 && enabledSources[0] === 'cursor';
  const includeCursor = !enabledSources || enabledSources.includes('cursor');

  // Check cursor auth early if cursor-only mode
  if (onlyCursor) {
    const credentials = loadCursorCredentials();
    if (!credentials) {
      console.log(pc.red("\n  Error: Cursor authentication required."));
      console.log(pc.gray("  Run 'tokscale cursor login' to authenticate with Cursor.\n"));
      process.exit(1);
    }
  }

  const dateRange = getDateRangeLabel(options);
  const title = dateRange 
    ? `Token Usage Report by Model (${dateRange})`
    : "Token Usage Report by Model";
  
  console.log(pc.cyan(`\n  ${title}`));
  if (options.benchmark) {
    console.log(pc.gray(`  Using: Rust native module v${getNativeVersion()}`));
  }
  console.log();

  // Start spinner for loading phase
  const spinner = createSpinner({ color: "cyan" });
  spinner.start(pc.gray("Loading data sources..."));

  // Filter out cursor for local parsing (it's synced separately via network)
  const localSources: SourceType[] = (enabledSources || ['opencode', 'claude', 'codex', 'gemini', 'cursor'])
    .filter(s => s !== 'cursor');

  // Two-phase parallel loading: network (Cursor + pricing) overlaps with local file parsing
  // If cursor-only, skip local parsing entirely
  const { fetcher, cursorSync, localMessages } = await loadDataSourcesParallel(
    onlyCursor ? [] : localSources,
    dateFilters
  );
  
  if (!localMessages && !onlyCursor) {
    spinner.error('Failed to parse local session files');
    process.exit(1);
  }

  spinner.update(pc.gray("Finalizing report..."));
  const startTime = performance.now();

  let report: ModelReport;
  try {
    const emptyMessages: ParsedMessages = { messages: [], opencodeCount: 0, claudeCount: 0, codexCount: 0, geminiCount: 0, processingTimeMs: 0 };
    report = await finalizeReportAsync({
      localMessages: localMessages || emptyMessages,
      pricing: fetcher.toPricingEntries(),
      includeCursor: includeCursor && cursorSync.synced,
      since: dateFilters.since,
      until: dateFilters.until,
      year: dateFilters.year,
    });
  } catch (e) {
    spinner.error(`Error: ${(e as Error).message}`);
    process.exit(1);
  }

  const processingTime = performance.now() - startTime;
  spinner.stop();

  if (report.entries.length === 0) {
    if (onlyCursor && !cursorSync.synced) {
      console.log(pc.yellow("  No Cursor data available."));
      console.log(pc.gray("  Run 'tokscale cursor login' to authenticate with Cursor.\n"));
    } else {
      console.log(pc.yellow("  No usage data found.\n"));
    }
    return;
  }

  // Create table
  const table = createUsageTable("Source/Model");

  for (const entry of report.entries) {
    const sourceLabel = getSourceLabel(entry.source);
    const modelDisplay = `${pc.dim(sourceLabel)} ${formatModelName(entry.model)}`;
    table.push(
      formatUsageRow(
        modelDisplay,
        [entry.model],
        entry.input,
        entry.output,
        entry.cacheWrite,
        entry.cacheRead,
        entry.cost
      )
    );
  }

  // Add totals row
  table.push(
    formatTotalsRow(
      report.totalInput,
      report.totalOutput,
      report.totalCacheWrite,
      report.totalCacheRead,
      report.totalCost
    )
  );

  console.log(table.toString());

  // Summary stats
  console.log(
    pc.gray(
      `\n  Total: ${formatNumber(report.totalMessages)} messages, ` +
        `${formatNumber(report.totalInput + report.totalOutput + report.totalCacheRead + report.totalCacheWrite)} tokens, ` +
        `${pc.green(formatCurrency(report.totalCost))}`
    )
  );

  if (options.benchmark) {
    console.log(pc.gray(`  Processing time: ${processingTime.toFixed(0)}ms (Rust) + ${report.processingTimeMs}ms (parsing)`));
    if (cursorSync.attempted) {
      if (cursorSync.synced) {
        console.log(pc.gray(`  Cursor: ${cursorSync.rows} usage events synced (full lifetime data)`));
      } else {
        console.log(pc.yellow(`  Cursor: sync failed - ${cursorSync.error}`));
      }
    }
  }

  console.log();
}

async function showMonthlyReport(options: FilterOptions & DateFilterOptions & { benchmark?: boolean }) {
  await ensureNativeModule();

  const dateRange = getDateRangeLabel(options);
  const title = dateRange 
    ? `Monthly Token Usage Report (${dateRange})`
    : "Monthly Token Usage Report";

  console.log(pc.cyan(`\n  ${title}`));
  if (options.benchmark) {
    console.log(pc.gray(`  Using: Rust native module v${getNativeVersion()}`));
  }
  console.log();

  // Start spinner for loading phase
  const spinner = createSpinner({ color: "cyan" });
  spinner.start(pc.gray("Loading data sources..."));

  const dateFilters = getDateFilters(options);
  const enabledSources = getEnabledSources(options);
  // Filter out cursor for local parsing (it's synced separately via network)
  const localSources: SourceType[] = (enabledSources || ['opencode', 'claude', 'codex', 'gemini', 'cursor'])
    .filter(s => s !== 'cursor');
  const includeCursor = !enabledSources || enabledSources.includes('cursor');

  // Two-phase parallel loading: network (Cursor + pricing) overlaps with local file parsing
  const { fetcher, cursorSync, localMessages } = await loadDataSourcesParallel(localSources, dateFilters);
  
  if (!localMessages) {
    spinner.error('Failed to parse local session files');
    process.exit(1);
  }

  spinner.update(pc.gray("Finalizing report..."));
  const startTime = performance.now();

  let report: MonthlyReport;
  try {
    report = await finalizeMonthlyReportAsync({
      localMessages,
      pricing: fetcher.toPricingEntries(),
      includeCursor: includeCursor && cursorSync.synced,
      since: dateFilters.since,
      until: dateFilters.until,
      year: dateFilters.year,
    });
  } catch (e) {
    spinner.error(`Error: ${(e as Error).message}`);
    process.exit(1);
  }

  const processingTime = performance.now() - startTime;
  spinner.stop();

  if (report.entries.length === 0) {
    console.log(pc.yellow("  No usage data found.\n"));
    return;
  }

  // Create table
  const table = createUsageTable("Month");

  for (const entry of report.entries) {
    table.push(
      formatUsageRow(
        entry.month,
        entry.models,
        entry.input,
        entry.output,
        entry.cacheWrite,
        entry.cacheRead,
        entry.cost
      )
    );
  }

  // Add totals row
  const totalInput = report.entries.reduce((sum, e) => sum + e.input, 0);
  const totalOutput = report.entries.reduce((sum, e) => sum + e.output, 0);
  const totalCacheRead = report.entries.reduce((sum, e) => sum + e.cacheRead, 0);
  const totalCacheWrite = report.entries.reduce((sum, e) => sum + e.cacheWrite, 0);

  table.push(
    formatTotalsRow(totalInput, totalOutput, totalCacheWrite, totalCacheRead, report.totalCost)
  );

  console.log(table.toString());
  console.log(pc.gray(`\n  Total Cost: ${pc.green(formatCurrency(report.totalCost))}`));

  if (options.benchmark) {
    console.log(pc.gray(`  Processing time: ${processingTime.toFixed(0)}ms (Rust) + ${report.processingTimeMs}ms (parsing)`));
    if (cursorSync.attempted) {
      if (cursorSync.synced) {
        console.log(pc.gray(`  Cursor: ${cursorSync.rows} usage events synced (full lifetime data)`));
      } else {
        console.log(pc.yellow(`  Cursor: sync failed - ${cursorSync.error}`));
      }
    }
  }

  console.log();
}

type JsonReportType = "models" | "monthly";

async function outputJsonReport(
  reportType: JsonReportType,
  options: FilterOptions & DateFilterOptions
) {
  await ensureNativeModule();

  const dateFilters = getDateFilters(options);
  const enabledSources = getEnabledSources(options);
  const onlyCursor = enabledSources?.length === 1 && enabledSources[0] === 'cursor';
  const includeCursor = !enabledSources || enabledSources.includes('cursor');
  const localSources: SourceType[] = (enabledSources || ['opencode', 'claude', 'codex', 'gemini', 'cursor'])
    .filter(s => s !== 'cursor');

  const { fetcher, cursorSync, localMessages } = await loadDataSourcesParallel(
    onlyCursor ? [] : localSources,
    dateFilters
  );
  
  if (!localMessages && !onlyCursor) {
    console.error(JSON.stringify({ error: "Failed to parse local session files" }));
    process.exit(1);
  }

  const emptyMessages: ParsedMessages = { messages: [], opencodeCount: 0, claudeCount: 0, codexCount: 0, geminiCount: 0, processingTimeMs: 0 };

  if (reportType === "models") {
    const report = await finalizeReportAsync({
      localMessages: localMessages || emptyMessages,
      pricing: fetcher.toPricingEntries(),
      includeCursor: includeCursor && cursorSync.synced,
      since: dateFilters.since,
      until: dateFilters.until,
      year: dateFilters.year,
    });
    console.log(JSON.stringify(report, null, 2));
  } else {
    const report = await finalizeMonthlyReportAsync({
      localMessages: localMessages || emptyMessages,
      pricing: fetcher.toPricingEntries(),
      includeCursor: includeCursor && cursorSync.synced,
      since: dateFilters.since,
      until: dateFilters.until,
      year: dateFilters.year,
    });
    console.log(JSON.stringify(report, null, 2));
  }
}

interface GraphCommandOptions extends FilterOptions, DateFilterOptions {
  output?: string;
  benchmark?: boolean;
}

async function handleGraphCommand(options: GraphCommandOptions) {
  await ensureNativeModule();

  // Start spinner for loading phase (only if outputting to file, not stdout)
  const spinner = options.output ? createSpinner({ color: "cyan" }) : null;
  spinner?.start(pc.gray("Loading data sources..."));

  const dateFilters = getDateFilters(options);
  const enabledSources = getEnabledSources(options);
  // Filter out cursor for local parsing (it's synced separately via network)
  const localSources: SourceType[] = (enabledSources || ['opencode', 'claude', 'codex', 'gemini', 'cursor'])
    .filter(s => s !== 'cursor');
  const includeCursor = !enabledSources || enabledSources.includes('cursor');

  // Two-phase parallel loading: network (Cursor + pricing) overlaps with local file parsing
  const { fetcher, cursorSync, localMessages } = await loadDataSourcesParallel(localSources, dateFilters);
  
  if (!localMessages) {
    spinner?.error('Failed to parse local session files');
    process.exit(1);
  }

  spinner?.update(pc.gray("Generating graph data..."));
  const startTime = performance.now();

  const data = await finalizeGraphAsync({
    localMessages,
    pricing: fetcher.toPricingEntries(),
    includeCursor: includeCursor && cursorSync.synced,
    since: dateFilters.since,
    until: dateFilters.until,
    year: dateFilters.year,
  });

  const processingTime = performance.now() - startTime;
  spinner?.stop();

  const jsonOutput = JSON.stringify(data, null, 2);

  // Output to file or stdout
  if (options.output) {
    fs.writeFileSync(options.output, jsonOutput, "utf-8");
    console.error(pc.green(`✓ Graph data written to ${options.output}`));
    console.error(
      pc.gray(
        `  ${data.contributions.length} days, ${data.summary.sources.length} sources, ${data.summary.models.length} models`
      )
    );
    console.error(pc.gray(`  Total: ${formatCurrency(data.summary.totalCost)}`));
    if (options.benchmark) {
      console.error(pc.gray(`  Processing time: ${processingTime.toFixed(0)}ms (Rust native)`));
      if (cursorSync.attempted) {
        if (cursorSync.synced) {
          console.error(pc.gray(`  Cursor: ${cursorSync.rows} usage events synced (full lifetime data)`));
        } else {
          console.error(pc.yellow(`  Cursor: sync failed - ${cursorSync.error}`));
        }
      }
    }
  } else {
    console.log(jsonOutput);
  }
}

function getSourceLabel(source: string): string {
  switch (source) {
    case "opencode":
      return "OpenCode";
    case "claude":
      return "Claude";
    case "codex":
      return "Codex";
    case "gemini":
      return "Gemini";
    case "cursor":
      return "Cursor";
    default:
      return source;
  }
}

// =============================================================================
// Cursor IDE Authentication
// =============================================================================

async function cursorLogin(): Promise<void> {
  const credentials = loadCursorCredentials();
  if (credentials) {
    console.log(pc.yellow("\n  Already logged in to Cursor."));
    console.log(pc.gray("  Run 'tokscale cursor logout' to sign out first.\n"));
    return;
  }

  console.log(pc.cyan("\n  Cursor IDE - Login\n"));
  console.log(pc.white("  To get your session token:"));
  console.log(pc.gray("  1. Open https://www.cursor.com/settings in your browser"));
  console.log(pc.gray("  2. Open Developer Tools (F12) > Network tab"));
  console.log(pc.gray("  3. Find any request to cursor.com/api"));
  console.log(pc.gray("  4. Copy the 'WorkosCursorSessionToken' cookie value"));
  console.log();

  // Read token from stdin
  const readline = await import("node:readline");
  const rl = readline.createInterface({
    input: process.stdin,
    output: process.stdout,
  });

  const token = await new Promise<string>((resolve) => {
    rl.question(pc.white("  Paste your session token: "), (answer) => {
      rl.close();
      resolve(answer.trim());
    });
  });

  if (!token) {
    console.log(pc.red("\n  No token provided. Login cancelled.\n"));
    return;
  }

  // Validate the token
  console.log(pc.gray("\n  Validating token..."));
  const validation = await validateCursorSession(token);

  if (!validation.valid) {
    console.log(pc.red(`\n  Invalid token: ${validation.error}`));
    console.log(pc.gray("  Please try again with a valid session token.\n"));
    return;
  }

  // Save credentials
  saveCursorCredentials({
    sessionToken: token,
    createdAt: new Date().toISOString(),
  });

  console.log(pc.green("\n  Success! Logged in to Cursor."));
  if (validation.membershipType) {
    console.log(pc.gray(`  Membership: ${validation.membershipType}`));
  }
  console.log(pc.gray("  Your usage data will now be included in reports.\n"));
}

async function cursorLogout(): Promise<void> {
  const credentials = loadCursorCredentials();

  if (!credentials) {
    console.log(pc.yellow("\n  Not logged in to Cursor.\n"));
    return;
  }

  const cleared = clearCursorCredentials();

  if (cleared) {
    console.log(pc.green("\n  Logged out from Cursor.\n"));
  } else {
    console.error(pc.red("\n  Failed to clear Cursor credentials.\n"));
    process.exit(1);
  }
}

async function cursorStatus(): Promise<void> {
  const credentials = loadCursorCredentials();

  if (!credentials) {
    console.log(pc.yellow("\n  Not logged in to Cursor."));
    console.log(pc.gray("  Run 'tokscale cursor login' to authenticate.\n"));
    return;
  }

  console.log(pc.cyan("\n  Cursor IDE - Status\n"));
  console.log(pc.gray("  Checking session validity..."));

  const validation = await validateCursorSession(credentials.sessionToken);

  if (validation.valid) {
    console.log(pc.green("  ✓ Session is valid"));
    if (validation.membershipType) {
      console.log(pc.white(`  Membership: ${validation.membershipType}`));
    }
    console.log(pc.gray(`  Logged in: ${new Date(credentials.createdAt).toLocaleDateString()}`));

    // Try to fetch usage to show summary
    try {
      const usage = await readCursorUsage();
      const totalCost = usage.byModel.reduce((sum, m) => sum + m.cost, 0);
      console.log(pc.gray(`  Models used: ${usage.byModel.length}`));
      console.log(pc.gray(`  Total usage events: ${usage.rows.length}`));
      console.log(pc.gray(`  Total cost: $${totalCost.toFixed(2)}`));
    } catch (e) {
      // Ignore fetch errors for status check
    }
  } else {
    console.log(pc.red(`  ✗ Session invalid: ${validation.error}`));
    console.log(pc.gray("  Run 'tokscale cursor login' to re-authenticate."));
  }

  console.log(pc.gray(`\n  Credentials: ${getCursorCredentialsPath()}\n`));
}

main().catch(console.error);
