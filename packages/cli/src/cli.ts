#!/usr/bin/env bun
/**
 * Tokscale CLI
 * Display OpenCode, Claude Code, Codex, Gemini, and Cursor usage with dynamic width tables
 * 
 * All heavy computation is done in the native Rust module.
 */

import { Command, Option } from "commander";
import { createRequire } from "module";
const require = createRequire(import.meta.url);
const pkg = require("../package.json") as { version: string };
import pc from "picocolors";
import { login, logout, whoami } from "./auth.js";
import { submit } from "./submit.js";
import { generateWrapped } from "./wrapped.js";

import {
  ensureCursorMigration,
  loadCursorCredentials,
  saveCursorCredentials,
  clearCursorCredentials,
  clearCursorCredentialsAndCache,
  isCursorLoggedIn,
  hasCursorUsageCache,
  listCursorAccounts,
  setActiveCursorAccount,
  removeCursorAccount,
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
import { spawn } from "node:child_process";
import { randomUUID } from "node:crypto";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { performance } from "node:perf_hooks";
import type { SourceType } from "./graph-types.js";
import type { TUIOptions, TabType } from "./tui/types/index.js";
import { loadSettings } from "./tui/config/settings.js";

type LaunchTUIFunction = (options?: TUIOptions) => Promise<void>;

let cachedTUILoader: LaunchTUIFunction | null = null;
let tuiLoadAttempted = false;

async function tryLoadTUI(): Promise<LaunchTUIFunction | null> {
  if (tuiLoadAttempted) return cachedTUILoader;
  tuiLoadAttempted = true;
  
  const isBun = typeof (globalThis as Record<string, unknown>).Bun !== "undefined";
  
  if (!isBun) {
    return null;
  }
  
  try {
    // Load OpenTUI preload to register Babel transform for TSX
    // This is needed for both dev mode (via bunfig.toml) and production
    // Use variable to prevent TypeScript from analyzing the module
    const preloadModule = "@opentui/solid/preload";
    await import(preloadModule);
    
    // Always load from source TSX - OpenTUI only works with Bun + preload
    // Calculate path to src/tui/index.tsx regardless of whether running from src or dist
    const currentPath = new URL(".", import.meta.url).pathname;
    const isFromDist = currentPath.includes("/dist/");
    const tuiPath = isFromDist
      ? new URL("../src/tui/index.tsx", import.meta.url).href
      : new URL("./tui/index.tsx", import.meta.url).href;
    const tuiModule = await import(tuiPath) as { launchTUI: LaunchTUIFunction };
    cachedTUILoader = tuiModule.launchTUI;
    return cachedTUILoader;
  } catch (error) {
    if (process.env.DEBUG) {
      console.error("TUI load error:", error);
    }
    return null;
  }
}

function showTUIUnavailableMessage(): void {
  console.log(pc.yellow("\n  TUI mode requires Bun runtime."));
  console.log(pc.gray("  OpenTUI's native modules are not compatible with Node.js."));
  console.log();
  console.log(pc.white("  Options:"));
  console.log(pc.gray("  • Use 'bunx tokscale' instead of 'npx tokscale'"));
  // console.log(pc.gray("  • Use '--light' flag for legacy CLI table output"));
  console.log(pc.gray("  • Use '--json' flag for JSON output"));
  console.log();
}

interface FilterOptions {
  opencode?: boolean;
  claude?: boolean;
  codex?: boolean;
  gemini?: boolean;
  cursor?: boolean;
  amp?: boolean;
  droid?: boolean;
  openclaw?: boolean;
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

function formatDateLocal(date: Date): string {
  const y = date.getFullYear();
  const m = String(date.getMonth() + 1).padStart(2, "0");
  const d = String(date.getDate()).padStart(2, "0");
  return `${y}-${m}-${d}`;
}

function getStartOfDayTimestamp(date: Date): number {
  const start = new Date(date.getFullYear(), date.getMonth(), date.getDate(), 0, 0, 0, 0);
  return start.getTime();
}

function getEndOfDayTimestamp(date: Date): number {
  const end = new Date(date.getFullYear(), date.getMonth(), date.getDate(), 23, 59, 59, 999);
  return end.getTime();
}

interface DateFilters {
  since?: string;
  until?: string;
  year?: string;
  sinceTs?: number;
  untilTs?: number;
}

function getDateFilters(options: DateFilterOptions): DateFilters {
  const today = new Date();
  
  // --today: just today (local timezone)
  if (options.today) {
    return {
      sinceTs: getStartOfDayTimestamp(today),
      untilTs: getEndOfDayTimestamp(today),
    };
  }
  
  // --week: last 7 days (local timezone)
  if (options.week) {
    const weekAgo = new Date(today);
    weekAgo.setDate(weekAgo.getDate() - 6);
    return {
      sinceTs: getStartOfDayTimestamp(weekAgo),
      untilTs: getEndOfDayTimestamp(today),
    };
  }
  
  // --month: current calendar month (local timezone)
  if (options.month) {
    const startOfMonth = new Date(today.getFullYear(), today.getMonth(), 1);
    return {
      sinceTs: getStartOfDayTimestamp(startOfMonth),
      untilTs: getEndOfDayTimestamp(today),
    };
  }
  
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

function getHeadlessRoots(homeDir: string): string[] {
  const override = process.env.TOKSCALE_HEADLESS_DIR;
  if (override && override.trim()) {
    return [override];
  }

  const roots = [
    path.join(homeDir, ".config", "tokscale", "headless"),
    path.join(homeDir, "Library", "Application Support", "tokscale", "headless"),
  ];

  return Array.from(new Set(roots));
}

function describePath(targetPath: string): string {
  return fs.existsSync(targetPath) ? targetPath : `${targetPath} (missing)`;
}

type HeadlessFormat = "json" | "jsonl";
type HeadlessSource = "codex";

const HEADLESS_SOURCES: HeadlessSource[] = ["codex"];

function normalizeHeadlessSource(source: string): HeadlessSource | null {
  const normalized = source.toLowerCase();
  return HEADLESS_SOURCES.includes(normalized as HeadlessSource)
    ? (normalized as HeadlessSource)
    : null;
}

function resolveHeadlessFormat(
  source: HeadlessSource,
  args: string[],
  override?: string
): HeadlessFormat {
  if (override === "json" || override === "jsonl") {
    return override;
  }

  return "jsonl";
}

function applyHeadlessDefaults(
  source: HeadlessSource,
  args: string[],
  format: HeadlessFormat,
  autoFlags: boolean
): string[] {
  if (!autoFlags) return args;

  const updated = [...args];

  if (source === "codex" && !updated.includes("--json")) {
    updated.push("--json");
  }

  return updated;
}

function buildHeadlessOutputPath(
  headlessRoots: string[],
  source: HeadlessSource,
  format: HeadlessFormat,
  outputPath?: string
): string {
  if (outputPath) {
    fs.mkdirSync(path.dirname(outputPath), { recursive: true });
    return outputPath;
  }

  const root = headlessRoots[0] || path.join(os.homedir(), ".config", "tokscale", "headless");
  const dir = path.join(root, source);
  fs.mkdirSync(dir, { recursive: true });

  const stamp = new Date().toISOString().replace(/[:.]/g, "-");
  const id = randomUUID().replace(/-/g, "").slice(0, 8);
  const filename = `${source}-${stamp}-${id}.${format}`;
  return path.join(dir, filename);
}

function printHeadlessHelp(): void {
  console.log("\n  Usage: tokscale headless codex [args...]");
  console.log("  Options:");
  console.log("    --format <json|jsonl>   Override output format");
  console.log("    --output <file>         Write captured output to file");
  console.log("    --no-auto-flags         Do not auto-add JSON output flags");
  console.log("\n  Examples:");
  console.log("    tokscale headless codex exec -m gpt-5");
  console.log();
}

async function runHeadlessCapture(argv: string[]): Promise<void> {
  const sourceArg = argv[1];
  if (!sourceArg || sourceArg === "--help" || sourceArg === "-h") {
    printHeadlessHelp();
    return;
  }

  const source = normalizeHeadlessSource(sourceArg);
  if (!source) {
    console.error(`\n  Error: Unknown headless source '${sourceArg}'.`);
    printHeadlessHelp();
    process.exit(1);
  }

  const rawArgs = argv.slice(2);
  let outputPath: string | undefined;
  let formatOverride: HeadlessFormat | undefined;
  let autoFlags = true;
  const cmdArgs: string[] = [];

  for (let i = 0; i < rawArgs.length; i += 1) {
    const arg = rawArgs[i];
    if (arg === "--") continue;
    if ((arg === "--help" || arg === "-h") && cmdArgs.length === 0) {
      printHeadlessHelp();
      return;
    }
    if (arg === "--output") {
      const value = rawArgs[i + 1];
      if (!value) {
        console.error("\n  Error: --output requires a file path.");
        process.exit(1);
      }
      outputPath = value;
      i += 1;
      continue;
    }
    if (arg === "--format") {
      const format = rawArgs[i + 1];
      if (!format) {
        console.error("\n  Error: --format requires a value (json or jsonl).");
        process.exit(1);
      }
      if (format !== "json" && format !== "jsonl") {
        console.error(`\n  Error: Invalid format '${format}'. Use json or jsonl.`);
        process.exit(1);
      }
      formatOverride = format as HeadlessFormat;
      i += 1;
      continue;
    }
    if (arg === "--no-auto-flags") {
      autoFlags = false;
      continue;
    }
    cmdArgs.push(arg);
  }

  if (cmdArgs.length === 0) {
    console.error("\n  Error: Missing CLI arguments to execute.");
    printHeadlessHelp();
    process.exit(1);
  }

  const format = resolveHeadlessFormat(source, cmdArgs, formatOverride);
  const finalArgs = applyHeadlessDefaults(source, cmdArgs, format, autoFlags);
  const headlessRoots = getHeadlessRoots(os.homedir());
  const output = buildHeadlessOutputPath(headlessRoots, source, format, outputPath);

  console.log(pc.cyan("\n  Headless capture"));
  console.log(pc.gray(`  source: ${source}`));
  console.log(pc.gray(`  output: ${output}`));
  console.log();

  const proc = spawn(source, finalArgs, {
    stdio: ["inherit", "pipe", "inherit"],
  });

  if (!proc.stdout) {
    console.error("\n  Error: Failed to capture stdout from command.");
    process.exit(1);
  }

  const outputStream = fs.createWriteStream(output, { encoding: "utf-8" });
  const outputFinished = new Promise<void>((resolve, reject) => {
    outputStream.on("finish", () => resolve());
    outputStream.on("error", reject);
  });
  proc.stdout.pipe(outputStream);
  let exitCode: number;
  try {
    exitCode = await new Promise<number>((resolve, reject) => {
      proc.on("error", reject);
      proc.on("close", (code) => resolve(code ?? 1));
    });
  } catch (err) {
    outputStream.destroy();
    const message = err instanceof Error ? err.message : String(err);
    console.error(`\n  Error: Failed to run '${source}': ${message}`);
    process.exit(1);
  }

  outputStream.end();

  try {
    await outputFinished;
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    console.error(`\n  Error: Failed to write headless output: ${message}`);
    process.exit(1);
  }

  if (exitCode !== 0) {
    process.exit(exitCode);
  }

  console.log(pc.green(`  Saved headless output to ${output}`));
  console.log();
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
    sinceTs: dateFilters.sinceTs,
    untilTs: dateFilters.untilTs,
  };
}

async function main() {
  const program = new Command();

  program
    .name("tokscale")
    .description("Tokscale - Track AI coding costs across OpenCode, Claude Code, Codex, Gemini, Cursor, and Amp")
    .version(pkg.version);

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
    .option("--amp", "Show only Amp usage")
    .option("--droid", "Show only Factory Droid usage")
    .option("--openclaw", "Show only OpenClaw usage")
    .option("--today", "Show only today's usage")
    .option("--week", "Show last 7 days")
    .option("--month", "Show current month")
    .option("--since <date>", "Start date (YYYY-MM-DD)")
    .option("--until <date>", "End date (YYYY-MM-DD)")
    .option("--year <year>", "Filter to specific year")
    .option("--benchmark", "Show processing time")
    .option("--no-spinner", "Disable spinner (for AI agents and scripts - keeps stdout clean)")
    .action(async (options) => {
      if (options.json) {
        await outputJsonReport("monthly", options);
      } else if (options.light) {
        await showMonthlyReport(options, { spinner: options.spinner });
      } else {
        const launchTUI = await tryLoadTUI();
        if (launchTUI) {
          await launchTUI(buildTUIOptions(options, "daily"));
        } else {
          showTUIUnavailableMessage();
          await showMonthlyReport(options, { spinner: options.spinner });
        }
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
    .option("--amp", "Show only Amp usage")
    .option("--droid", "Show only Factory Droid usage")
    .option("--openclaw", "Show only OpenClaw usage")
    .option("--today", "Show only today's usage")
    .option("--week", "Show last 7 days")
    .option("--month", "Show current month")
    .option("--since <date>", "Start date (YYYY-MM-DD)")
    .option("--until <date>", "End date (YYYY-MM-DD)")
    .option("--year <year>", "Filter to specific year")
    .option("--benchmark", "Show processing time")
    .option("--no-spinner", "Disable spinner (for AI agents and scripts - keeps stdout clean)")
    .action(async (options) => {
      if (options.json) {
        await outputJsonReport("models", options);
      } else if (options.light) {
        await showModelReport(options, { spinner: options.spinner });
      } else {
        const launchTUI = await tryLoadTUI();
        if (launchTUI) {
          await launchTUI(buildTUIOptions(options, "model"));
        } else {
          showTUIUnavailableMessage();
          await showModelReport(options, { spinner: options.spinner });
        }
      }
    });

  program
    .command("sources")
    .description("Show local scan locations and session counts")
    .option("--json", "Output as JSON (for scripting)")
    .action(async (options) => {
      const homeDir = os.homedir();
      const headlessRoots = getHeadlessRoots(homeDir);

      // Define all session paths
      const opencodeSessions = path.join(homeDir, ".local", "share", "opencode", "storage", "message");
      const claudeSessions = path.join(homeDir, ".claude", "projects");
      const codexHome = process.env.CODEX_HOME || path.join(homeDir, ".codex");
      const codexSessions = path.join(codexHome, "sessions");
      const geminiSessions = path.join(homeDir, ".gemini", "tmp");
      const ampSessions = path.join(homeDir, ".local", "share", "amp", "threads");
      const droidSessions = path.join(homeDir, ".factory", "sessions");
      const openclawSessions = path.join(homeDir, ".openclaw", "agents");
      const openclawLegacyPaths = [
        path.join(homeDir, ".clawdbot", "agents"),
        path.join(homeDir, ".moltbot", "agents"),
        path.join(homeDir, ".moldbot", "agents"),
      ];

      let localMessages: ParsedMessages | null = null;
      try {
        localMessages = await parseLocalSourcesAsync({
          homeDir,
          sources: ["opencode", "claude", "codex", "gemini", "amp", "droid", "openclaw"],
        });
      } catch (e) {
        console.error(`Error: ${(e as Error).message}`);
        process.exit(1);
      }

      const headlessCounts = {
        codex: 0,
      };

      for (const message of localMessages.messages) {
        if (message.agent === "headless" && message.source === "codex") {
          headlessCounts.codex += 1;
        }
      }

      const sourceRows: Array<{
        source: SourceType;
        label: string;
        sessionsPath: string;
        legacyPaths?: string[];
        messageCount: number;
        headlessSupported: boolean;
        headlessPaths: string[];
        headlessMessageCount: number;
      }> = [
        {
          source: "opencode",
          label: "OpenCode",
          sessionsPath: opencodeSessions,
          messageCount: localMessages.opencodeCount,
          headlessSupported: false,
          headlessPaths: [],
          headlessMessageCount: 0,
        },
        {
          source: "claude",
          label: "Claude Code",
          sessionsPath: claudeSessions,
          messageCount: localMessages.claudeCount,
          headlessSupported: false,
          headlessPaths: [],
          headlessMessageCount: 0,
        },
        {
          source: "codex",
          label: "Codex CLI",
          sessionsPath: codexSessions,
          headlessPaths: headlessRoots.map((root) => path.join(root, "codex")),
          messageCount: localMessages.codexCount,
          headlessMessageCount: headlessCounts.codex,
          headlessSupported: true,
        },
        {
          source: "gemini",
          label: "Gemini CLI",
          sessionsPath: geminiSessions,
          messageCount: localMessages.geminiCount,
          headlessSupported: false,
          headlessPaths: [],
          headlessMessageCount: 0,
        },
        {
          source: "cursor",
          label: "Cursor IDE",
          sessionsPath: path.join(homeDir, ".config", "tokscale", "cursor-cache"),
          messageCount: 0, // Cursor uses API sync, not local sessions
          headlessSupported: false,
          headlessPaths: [],
          headlessMessageCount: 0,
        },
        {
          source: "amp",
          label: "Amp",
          sessionsPath: ampSessions,
          messageCount: localMessages.ampCount,
          headlessSupported: false,
          headlessPaths: [],
          headlessMessageCount: 0,
        },
        {
          source: "droid",
          label: "Droid",
          sessionsPath: droidSessions,
          messageCount: localMessages.droidCount,
          headlessSupported: false,
          headlessPaths: [],
          headlessMessageCount: 0,
        },
        {
          source: "openclaw",
          label: "OpenClaw",
          sessionsPath: openclawSessions,
          legacyPaths: openclawLegacyPaths,
          messageCount: localMessages.openclawCount,
          headlessSupported: false,
          headlessPaths: [],
          headlessMessageCount: 0,
        },
      ];

      if (options.json) {
        const payload = {
          headlessRoots,
          sources: sourceRows.map((row) => ({
            source: row.source,
            label: row.label,
            sessionsPath: row.sessionsPath,
            sessionsPathExists: fs.existsSync(row.sessionsPath),
            legacyPaths: row.legacyPaths
              ? row.legacyPaths.map((legacyPath) => ({
                  path: legacyPath,
                  exists: fs.existsSync(legacyPath),
                }))
              : [],
            messageCount: row.messageCount,
            headlessSupported: row.headlessSupported,
            headlessPaths: row.headlessSupported
              ? row.headlessPaths.map((headlessPath) => ({
                  path: headlessPath,
                  exists: fs.existsSync(headlessPath),
                }))
              : [],
            headlessMessageCount: row.headlessSupported ? row.headlessMessageCount : 0,
          })),
          note: "Headless capture is supported for Codex CLI only.",
        };
        console.log(JSON.stringify(payload, null, 2));
        return;
      }

      console.log(pc.cyan("\n  Local sources & session counts"));
      console.log(pc.gray(`  Headless roots: ${headlessRoots.join(", ")}`));
      console.log();

      for (const row of sourceRows) {
        console.log(pc.white(`  ${row.label}`));
        console.log(pc.gray(`  sessions: ${describePath(row.sessionsPath)}`));
        if (row.legacyPaths && row.legacyPaths.length > 0) {
          console.log(
            pc.gray(`  legacy: ${row.legacyPaths.map(describePath).join(", ")}`)
          );
        }
        if (row.headlessSupported) {
          console.log(
            pc.gray(
              `  headless: ${row.headlessPaths.map(describePath).join(", ")}`
            )
          );
          console.log(
            pc.gray(
              `  messages: ${formatNumber(row.messageCount)} (headless: ${formatNumber(
                row.headlessMessageCount
              )})`
            )
          );
        } else {
          console.log(pc.gray(`  messages: ${formatNumber(row.messageCount)}`));
        }
        console.log();
      }

      console.log(
        pc.gray(
          "  Note: Headless capture is supported for Codex CLI only."
        )
      );
      console.log();
    });

  program
    .command("headless")
    .description("Run a CLI in headless mode and capture stdout")
    .argument("<source>", "Source CLI to capture (currently only 'codex' is supported)")
    .argument("[args...]", "Arguments passed to the CLI");

  program
    .command("graph")
    .description("Export contribution graph data as JSON")
    .option("--output <file>", "Write to file instead of stdout")
    .option("--opencode", "Include only OpenCode data")
    .option("--claude", "Include only Claude Code data")
    .option("--codex", "Include only Codex CLI data")
    .option("--gemini", "Include only Gemini CLI data")
    .option("--cursor", "Include only Cursor IDE data")
    .option("--amp", "Include only Amp data")
    .option("--droid", "Include only Factory Droid data")
    .option("--openclaw", "Include only OpenClaw data")
    .option("--today", "Show only today's usage")
    .option("--week", "Show last 7 days")
    .option("--month", "Show current month")
    .option("--since <date>", "Start date (YYYY-MM-DD)")
    .option("--until <date>", "End date (YYYY-MM-DD)")
    .option("--year <year>", "Filter to specific year")
    .option("--benchmark", "Show processing time")
    .option("--no-spinner", "Disable spinner (for AI agents and scripts - keeps stdout clean)")
    .action(async (options) => {
      await handleGraphCommand(options);
    });

  program
    .command("wrapped")
    .description("Generate Wrapped shareable image")
    .option("--output <file>", "Output file path (default: tokscale-<year>-wrapped.png)")
    .option("--year <year>", "Year to generate (default: current year)")
    .option("--opencode", "Include only OpenCode data")
    .option("--claude", "Include only Claude Code data")
    .option("--codex", "Include only Codex CLI data")
    .option("--gemini", "Include only Gemini CLI data")
    .option("--cursor", "Include only Cursor IDE data")
    .option("--amp", "Include only Amp data")
    .option("--droid", "Include only Factory Droid data")
    .option("--openclaw", "Include only OpenClaw data")
    .option("--no-spinner", "Disable loading spinner (for scripting)")
    .option("--short", "Display total tokens in abbreviated format (e.g., 7.14B)")
    .addOption(new Option("--agents", "Show Top OpenCode Agents (default)").conflicts("clients"))
    .addOption(new Option("--clients", "Show Top Clients instead of Top OpenCode Agents").conflicts("agents"))
    .option("--disable-pinned", "Disable pinning of Sisyphus agents in rankings")
    .action(async (options) => {
      await handleWrappedCommand(options);
    });

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
    .option("--amp", "Include only Amp data")
    .option("--droid", "Include only Factory Droid data")
    .option("--openclaw", "Include only OpenClaw data")
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
        amp: options.amp,
        droid: options.droid,
        openclaw: options.openclaw,
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
    .option("--amp", "Show only Amp usage")
    .option("--droid", "Show only Factory Droid usage")
    .option("--openclaw", "Show only OpenClaw usage")
    .option("--today", "Show only today's usage")
    .option("--week", "Show last 7 days")
    .option("--month", "Show current month")
    .option("--since <date>", "Start date (YYYY-MM-DD)")
    .option("--until <date>", "End date (YYYY-MM-DD)")
    .option("--year <year>", "Filter to specific year")
    .action(async (options) => {
      const launchTUI = await tryLoadTUI();
      if (launchTUI) {
        await launchTUI(buildTUIOptions(options));
      } else {
        showTUIUnavailableMessage();
        process.exit(1);
      }
    });

  program
    .command("pricing <model-id>")
    .description("Look up pricing for a model")
    .option("--json", "Output as JSON")
    .option("--provider <source>", "Force pricing source: 'litellm' or 'openrouter'")
    .option("--no-spinner", "Disable spinner (for AI agents and scripts - keeps stdout clean)")
    .action(async (modelId: string, options: { json?: boolean; provider?: string; spinner?: boolean }) => {
      await handlePricingCommand(modelId, options);
    });

  const cursorCommand = program
    .command("cursor")
    .description("Cursor IDE integration commands");

  cursorCommand
    .command("login")
    .description("Login to Cursor (paste your session token)")
    .option("--name <name>", "Label for this Cursor account (e.g., work, personal)")
    .action(async (options: { name?: string }) => {
      ensureCursorMigration();
      await cursorLogin(options);
    });

  cursorCommand
    .command("logout")
    .description("Logout from a Cursor account")
    .option("--name <name>", "Account label or id")
    .option("--all", "Logout from all Cursor accounts")
    .option("--purge-cache", "Also delete cached Cursor usage for the logged-out account(s)")
    .action(async (options: { name?: string; all?: boolean; purgeCache?: boolean }) => {
      ensureCursorMigration();
      await cursorLogout(options);
    });

  cursorCommand
    .command("status")
    .description("Check Cursor authentication status")
    .option("--name <name>", "Account label or id")
    .action(async (options: { name?: string }) => {
      ensureCursorMigration();
      await cursorStatus(options);
    });

  cursorCommand
    .command("accounts")
    .description("List saved Cursor accounts")
    .option("--json", "Output as JSON")
    .action(async (options: { json?: boolean }) => {
      ensureCursorMigration();
      const accounts = listCursorAccounts();
      if (options.json) {
        console.log(JSON.stringify({ accounts }, null, 2));
        return;
      }

      if (accounts.length === 0) {
        console.log(pc.yellow("\n  No saved Cursor accounts.\n"));
        return;
      }

      console.log(pc.cyan("\n  Cursor IDE - Accounts\n"));
      for (const acct of accounts) {
        const name = acct.label ? `${acct.label} ${pc.gray(`(${acct.id})`)}` : acct.id;
        console.log(`  ${acct.isActive ? pc.green("*") : pc.gray("-")} ${name}`);
      }
      console.log();
    });

  cursorCommand
    .command("switch")
    .description("Switch active Cursor account")
    .argument("<name>", "Account label or id")
    .action(async (name: string) => {
      ensureCursorMigration();
      const result = setActiveCursorAccount(name);
      if (!result.ok) {
        console.log(pc.red(`\n  Error: ${result.error}\n`));
        process.exit(1);
      }
      console.log(pc.green(`\n  Active Cursor account set to ${pc.bold(name)}\n`));
    });

  // Check if a subcommand was provided
  const args = process.argv.slice(2);
  if (args[0] === "headless") {
    await runHeadlessCapture(args);
    return;
  }
  const firstArg = args[0] || '';
  // Global flags should go to main program
  const isGlobalFlag = ['--help', '-h', '--version', '-V'].includes(firstArg);
  const hasSubcommand = args.length > 0 && !firstArg.startsWith('-');
  const knownCommands = ['monthly', 'models', 'sources', 'headless', 'graph', 'wrapped', 'login', 'logout', 'whoami', 'submit', 'cursor', 'tui', 'pricing', 'help'];
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
      .option("--amp", "Show only Amp usage")
      .option("--droid", "Show only Factory Droid usage")
      .option("--openclaw", "Show only OpenClaw usage")
      .option("--today", "Show only today's usage")
      .option("--week", "Show last 7 days")
      .option("--month", "Show current month")
      .option("--since <date>", "Start date (YYYY-MM-DD)")
      .option("--until <date>", "End date (YYYY-MM-DD)")
      .option("--year <year>", "Filter to specific year")
      .option("--benchmark", "Show processing time")
      .option("--no-spinner", "Disable spinner (for AI agents and scripts - keeps stdout clean)")
      .parse();
    
    const opts = defaultProgram.opts();
    if (opts.json) {
      await outputJsonReport("models", opts);
    } else if (opts.light) {
      await showModelReport(opts, { spinner: opts.spinner });
    } else {
      const launchTUI = await tryLoadTUI();
      if (launchTUI) {
        await launchTUI(buildTUIOptions(opts));
      } else {
        showTUIUnavailableMessage();
        await showModelReport(opts, { spinner: opts.spinner });
      }
    }
  }
}

function getEnabledSources(options: FilterOptions): SourceType[] | undefined {
  const hasFilter = options.opencode || options.claude || options.codex || options.gemini || options.cursor || options.amp || options.droid || options.openclaw;
  if (!hasFilter) return undefined; // All sources

  const sources: SourceType[] = [];
  if (options.opencode) sources.push("opencode");
  if (options.claude) sources.push("claude");
  if (options.codex) sources.push("codex");
  if (options.gemini) sources.push("gemini");
  if (options.cursor) sources.push("cursor");
  if (options.amp) sources.push("amp");
  if (options.droid) sources.push("droid");
  if (options.openclaw) sources.push("openclaw");
  return sources;
}





/**
 * Sync Cursor usage data from API to local cache.
 * Only attempts sync if user is authenticated with Cursor.
 */
async function syncCursorData(): Promise<CursorSyncResult> {
  if (!isCursorLoggedIn()) {
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
  cursorSync: CursorSyncResult;
  localMessages: ParsedMessages | null;
}

async function loadDataSourcesParallel(
  localSources: SourceType[],
  dateFilters: { since?: string; until?: string; year?: string },
  onPhase?: (phase: string) => void
): Promise<LoadedDataSources> {
  const shouldParseLocal = localSources.length > 0;

  const [cursorResult, localResult] = await Promise.allSettled([
    syncCursorData(),
    shouldParseLocal
      ? parseLocalSourcesAsync({
          sources: localSources.filter(s => s !== 'cursor'),
          since: dateFilters.since,
          until: dateFilters.until,
          year: dateFilters.year,
        })
      : Promise.resolve(null),
  ]);

  const cursorSync: CursorSyncResult = cursorResult.status === 'fulfilled'
    ? cursorResult.value
    : { attempted: true, synced: false, rows: 0, error: 'Cursor sync failed' };

  const localMessages: ParsedMessages | null = localResult.status === 'fulfilled'
    ? localResult.value
    : null;

  return { cursorSync, localMessages };
}

async function showModelReport(options: FilterOptions & DateFilterOptions & { benchmark?: boolean }, extraOptions?: { spinner?: boolean }) {
  const dateFilters = getDateFilters(options);
  const enabledSources = getEnabledSources(options);
  const onlyCursor = enabledSources?.length === 1 && enabledSources[0] === 'cursor';
  const includeCursor = !enabledSources || enabledSources.includes('cursor');

  // Check cursor auth early if cursor-only mode
  if (onlyCursor) {
    if (!isCursorLoggedIn() && !hasCursorUsageCache()) {
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

  const useSpinner = extraOptions?.spinner !== false;
  const spinner = useSpinner ? createSpinner({ color: "cyan" }) : null;

  const localSources: SourceType[] = (enabledSources || ['opencode', 'claude', 'codex', 'gemini', 'cursor', 'amp', 'droid', 'openclaw'])
    .filter(s => s !== 'cursor');

  spinner?.start(pc.gray("Scanning session data..."));

  const { cursorSync, localMessages } = await loadDataSourcesParallel(
    onlyCursor ? [] : localSources,
    dateFilters,
    (phase) => spinner?.update(phase)
  );

  if (includeCursor && cursorSync.attempted && cursorSync.error) {
    // Don't block report generation; just warn about partial Cursor sync.
    console.log(pc.yellow(`  Cursor sync warning: ${cursorSync.error}`));
  }
  
  if (!localMessages && !onlyCursor) {
    if (spinner) {
      spinner.error('Failed to parse local session files');
    } else {
      console.error('Failed to parse local session files');
    }
    process.exit(1);
  }

  spinner?.update(pc.gray("Finalizing report..."));
  const startTime = performance.now();

  let report: ModelReport;
  try {
    const emptyMessages: ParsedMessages = { messages: [], opencodeCount: 0, claudeCount: 0, codexCount: 0, geminiCount: 0, ampCount: 0, droidCount: 0, openclawCount: 0, processingTimeMs: 0 };
    report = await finalizeReportAsync({
      localMessages: localMessages || emptyMessages,
      includeCursor: includeCursor && (cursorSync.synced || hasCursorUsageCache()),
      since: dateFilters.since,
      until: dateFilters.until,
      year: dateFilters.year,
    });
  } catch (e) {
    if (spinner) {
      spinner.error(`Error: ${(e as Error).message}`);
    } else {
      console.error(`Error: ${(e as Error).message}`);
    }
    process.exit(1);
  }

  const processingTime = performance.now() - startTime;
  spinner?.stop();

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
  
  const settings = loadSettings();
  const filteredEntries = settings.includeUnusedModels 
    ? report.entries 
    : report.entries.filter(e => e.input + e.output + e.cacheRead + e.cacheWrite > 0);

  for (const entry of filteredEntries) {
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

async function showMonthlyReport(options: FilterOptions & DateFilterOptions & { benchmark?: boolean }, extraOptions?: { spinner?: boolean }) {
  const dateRange = getDateRangeLabel(options);
  const title = dateRange 
    ? `Monthly Token Usage Report (${dateRange})`
    : "Monthly Token Usage Report";

  console.log(pc.cyan(`\n  ${title}`));
  if (options.benchmark) {
    console.log(pc.gray(`  Using: Rust native module v${getNativeVersion()}`));
  }
  console.log();

  const useSpinner = extraOptions?.spinner !== false;
  const spinner = useSpinner ? createSpinner({ color: "cyan" }) : null;

  const dateFilters = getDateFilters(options);
  const enabledSources = getEnabledSources(options);
  const localSources: SourceType[] = (enabledSources || ['opencode', 'claude', 'codex', 'gemini', 'cursor', 'amp', 'droid', 'openclaw'])
    .filter(s => s !== 'cursor');
  const includeCursor = !enabledSources || enabledSources.includes('cursor');

  spinner?.start(pc.gray("Scanning session data..."));

  const { cursorSync, localMessages } = await loadDataSourcesParallel(
    localSources,
    dateFilters,
    (phase) => spinner?.update(phase)
  );
  
  if (!localMessages) {
    if (spinner) {
      spinner.error('Failed to parse local session files');
    } else {
      console.error('Failed to parse local session files');
    }
    process.exit(1);
  }

  spinner?.update(pc.gray("Finalizing report..."));
  const startTime = performance.now();

  let report: MonthlyReport;
  try {
    report = await finalizeMonthlyReportAsync({
      localMessages,
      includeCursor: includeCursor && (cursorSync.synced || hasCursorUsageCache()),
      since: dateFilters.since,
      until: dateFilters.until,
      year: dateFilters.year,
    });
  } catch (e) {
    if (spinner) {
      spinner.error(`Error: ${(e as Error).message}`);
    } else {
      console.error(`Error: ${(e as Error).message}`);
    }
    process.exit(1);
  }

  const processingTime = performance.now() - startTime;
  spinner?.stop();

  if (report.entries.length === 0) {
    console.log(pc.yellow("  No usage data found.\n"));
    return;
  }

  // Create table
  const table = createUsageTable("Month");

  const settings = loadSettings();
  const filteredEntries = settings.includeUnusedModels
    ? report.entries
    : report.entries.filter(e => e.input + e.output + e.cacheRead + e.cacheWrite > 0);

  for (const entry of filteredEntries) {
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
  const dateFilters = getDateFilters(options);
  const enabledSources = getEnabledSources(options);
  const onlyCursor = enabledSources?.length === 1 && enabledSources[0] === 'cursor';
  const includeCursor = !enabledSources || enabledSources.includes('cursor');
  const localSources: SourceType[] = (enabledSources || ['opencode', 'claude', 'codex', 'gemini', 'cursor', 'amp', 'droid', 'openclaw'])
    .filter(s => s !== 'cursor');

  const { cursorSync, localMessages } = await loadDataSourcesParallel(
    onlyCursor ? [] : localSources,
    dateFilters
  );
  
  if (!localMessages && !onlyCursor) {
    console.error(JSON.stringify({ error: "Failed to parse local session files" }));
    process.exit(1);
  }

  const emptyMessages: ParsedMessages = { messages: [], opencodeCount: 0, claudeCount: 0, codexCount: 0, geminiCount: 0, ampCount: 0, droidCount: 0, openclawCount: 0, processingTimeMs: 0 };

  if (reportType === "models") {
    const report = await finalizeReportAsync({
      localMessages: localMessages || emptyMessages,
      includeCursor: includeCursor && (cursorSync.synced || hasCursorUsageCache()),
      since: dateFilters.since,
      until: dateFilters.until,
      year: dateFilters.year,
    });
    console.log(JSON.stringify(report, null, 2));
  } else {
    const report = await finalizeMonthlyReportAsync({
      localMessages: localMessages || emptyMessages,
      includeCursor: includeCursor && (cursorSync.synced || hasCursorUsageCache()),
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
  spinner?: boolean;
}

async function handleGraphCommand(options: GraphCommandOptions) {
  const useSpinner = options.output && options.spinner !== false;
  const spinner = useSpinner ? createSpinner({ color: "cyan" }) : null;

  const dateFilters = getDateFilters(options);
  const enabledSources = getEnabledSources(options);
  const localSources: SourceType[] = (enabledSources || ['opencode', 'claude', 'codex', 'gemini', 'cursor', 'amp', 'droid', 'openclaw'])
    .filter(s => s !== 'cursor');
  const includeCursor = !enabledSources || enabledSources.includes('cursor');

  spinner?.start(pc.gray("Scanning session data..."));

  const { cursorSync, localMessages } = await loadDataSourcesParallel(
    localSources,
    dateFilters,
    (phase) => spinner?.update(phase)
  );
  
  if (!localMessages) {
    spinner?.error('Failed to parse local session files');
    process.exit(1);
  }

  spinner?.update(pc.gray("Generating graph data..."));
  const startTime = performance.now();

  const data = await finalizeGraphAsync({
    localMessages,
    includeCursor: includeCursor && (cursorSync.synced || hasCursorUsageCache()),
    since: dateFilters.since,
    until: dateFilters.until,
    year: dateFilters.year,
    sinceTs: dateFilters.sinceTs,
    untilTs: dateFilters.untilTs,
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

interface WrappedCommandOptions extends FilterOptions {
  output?: string;
  year?: string;
  spinner?: boolean;
  short?: boolean;
  agents?: boolean;
  clients?: boolean;
  disablePinned?: boolean;
}

async function handleWrappedCommand(options: WrappedCommandOptions) {
  const useSpinner = options.spinner !== false;
  const spinner = useSpinner ? createSpinner({ color: "cyan" }) : null;
  const currentYear = new Date().getFullYear().toString();
  const year = options.year || currentYear;
  spinner?.start(pc.gray(`Generating your ${year} Wrapped...`));

  try {
    const enabledSources = getEnabledSources(options);
    const outputPath = await generateWrapped({
      output: options.output,
      year,
      sources: enabledSources,
      short: options.short,
      includeAgents: !options.clients,
      pinSisyphus: !options.disablePinned,
    });

    spinner?.stop();
    console.log(pc.green(`\n  ✓ Your Tokscale Wrapped image is ready!`));
    console.log(pc.white(`  ${outputPath}`));
    console.log();
    console.log(pc.gray("  Share it on Twitter/X with #TokscaleWrapped"));
    console.log();
  } catch (error) {
    if (spinner) {
      spinner.error(`Failed to generate wrapped: ${(error as Error).message}`);
    } else {
      console.error(pc.red(`Failed to generate wrapped: ${(error as Error).message}`));
    }
    process.exit(1);
  }
}

async function handlePricingCommand(modelId: string, options: { json?: boolean; provider?: string; spinner?: boolean }) {
  const validProviders = ["litellm", "openrouter"];
  if (options.provider && !validProviders.includes(options.provider.toLowerCase())) {
    console.log(pc.red(`\n  Invalid provider: ${options.provider}`));
    console.log(pc.gray(`  Valid providers: ${validProviders.join(", ")}\n`));
    process.exit(1);
  }

  const useSpinner = options.spinner !== false;
  const spinner = useSpinner ? createSpinner({ color: "cyan" }) : null;
  const providerLabel = options.provider ? ` from ${options.provider}` : "";
  spinner?.start(pc.gray(`Fetching pricing data${providerLabel}...`));

  let core: typeof import("@tokscale/core");
  try {
    const mod = await import("@tokscale/core");
    core = (mod.default ?? mod) as typeof import("@tokscale/core");
  } catch (importErr) {
    spinner?.stop();
    const errorMsg = (importErr as Error).message || "Unknown error";
    if (options.json) {
      console.log(JSON.stringify({ error: "Native module not available", details: errorMsg }, null, 2));
    } else {
      console.log(pc.red(`\n  Native module not available: ${errorMsg}`));
      console.log(pc.gray("  Run 'bun run build:core' to build the native module.\n"));
    }
    process.exit(1);
  }

  try {
    const provider = options.provider?.toLowerCase() || undefined;
    const nativeResult = await core.lookupPricing(modelId, provider);
    spinner?.stop();

    const result = {
      matchedKey: nativeResult.matchedKey,
      source: nativeResult.source as "litellm" | "openrouter",
      pricing: {
        input_cost_per_token: nativeResult.pricing.inputCostPerToken,
        output_cost_per_token: nativeResult.pricing.outputCostPerToken,
        cache_read_input_token_cost: nativeResult.pricing.cacheReadInputTokenCost,
        cache_creation_input_token_cost: nativeResult.pricing.cacheCreationInputTokenCost,
      },
    };

    if (options.json) {
      console.log(JSON.stringify({
        modelId,
        matchedKey: result.matchedKey,
        source: result.source,
        pricing: {
          inputCostPerToken: result.pricing.input_cost_per_token ?? 0,
          outputCostPerToken: result.pricing.output_cost_per_token ?? 0,
          cacheReadInputTokenCost: result.pricing.cache_read_input_token_cost,
          cacheCreationInputTokenCost: result.pricing.cache_creation_input_token_cost,
        },
      }, null, 2));
    } else {
      const sourceLabel = result.source.toLowerCase() === "litellm" ? pc.blue("LiteLLM") : pc.magenta("OpenRouter");
      const inputCost = result.pricing.input_cost_per_token ?? 0;
      const outputCost = result.pricing.output_cost_per_token ?? 0;
      const cacheReadCost = result.pricing.cache_read_input_token_cost;
      const cacheWriteCost = result.pricing.cache_creation_input_token_cost;

      console.log(pc.cyan(`\n  Pricing for: ${pc.white(modelId)}`));
      console.log(pc.gray(`  Matched key: ${result.matchedKey}`));
      console.log(pc.gray(`  Source: `) + sourceLabel);
      console.log();
      console.log(pc.white(`  Input:  `) + formatPricePerMillion(inputCost));
      console.log(pc.white(`  Output: `) + formatPricePerMillion(outputCost));
      if (cacheReadCost !== undefined) {
        console.log(pc.white(`  Cache Read:  `) + formatPricePerMillion(cacheReadCost));
      }
      if (cacheWriteCost !== undefined) {
        console.log(pc.white(`  Cache Write: `) + formatPricePerMillion(cacheWriteCost));
      }
      console.log();
    }
  } catch (err) {
    spinner?.stop();
    const errorMsg = (err as Error).message || "Unknown error";
    
    // Check if this is a "model not found" error from Rust or a different error
    const isModelNotFound = errorMsg.toLowerCase().includes("not found") || 
                            errorMsg.toLowerCase().includes("no pricing");
    
    if (options.json) {
      if (isModelNotFound) {
        console.log(JSON.stringify({ error: "Model not found", modelId }, null, 2));
      } else {
        console.log(JSON.stringify({ error: errorMsg, modelId }, null, 2));
      }
    } else {
      if (isModelNotFound) {
        console.log(pc.red(`\n  Model not found: ${modelId}\n`));
      } else {
        console.log(pc.red(`\n  Error looking up pricing: ${errorMsg}\n`));
      }
    }
    process.exit(1);
  }
}

function formatPricePerMillion(costPerToken: number): string {
  const perMillion = costPerToken * 1_000_000;
  return pc.green(`$${perMillion.toFixed(2)}`) + pc.gray(" / 1M tokens");
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
    case "amp":
      return "Amp";
    case "droid":
      return "Droid";
    default:
      return source;
  }
}

// =============================================================================
// Cursor IDE Authentication
// =============================================================================

async function cursorLogin(options: { name?: string } = {}): Promise<void> {
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

  // Save credentials (multi-account)
  let savedAccountId: string;
  try {
    const saved = saveCursorCredentials(
      {
        sessionToken: token,
        createdAt: new Date().toISOString(),
      },
      { label: options.name }
    );
    savedAccountId = saved.accountId;
  } catch (e) {
    console.log(pc.red(`\n  Failed to save credentials: ${(e as Error).message}\n`));
    return;
  }

  console.log(pc.green("\n  Success! Logged in to Cursor."));
  if (options.name) {
    console.log(pc.gray(`  Account: ${options.name} (${savedAccountId})`));
  } else {
    console.log(pc.gray(`  Account ID: ${savedAccountId}`));
  }
  if (validation.membershipType) {
    console.log(pc.gray(`  Membership: ${validation.membershipType}`));
  }
  console.log(pc.gray("  Your usage data will now be included in reports.\n"));
}

async function cursorLogout(options: { name?: string; all?: boolean; purgeCache?: boolean } = {}): Promise<void> {
  if (!isCursorLoggedIn()) {
    console.log(pc.yellow("\n  Not logged in to Cursor.\n"));
    return;
  }

  if (options.all) {
    const cleared = options.purgeCache ? clearCursorCredentialsAndCache({ purgeCache: true }) : clearCursorCredentialsAndCache();
    if (cleared) {
      console.log(pc.green("\n  Logged out from all Cursor accounts.\n"));
      return;
    }
    console.error(pc.red("\n  Failed to clear Cursor credentials.\n"));
    process.exit(1);
  }

  const target = options.name || listCursorAccounts().find((a) => a.isActive)?.id;
  if (!target) {
    console.log(pc.yellow("\n  No saved Cursor accounts.\n"));
    return;
  }

  const removed = removeCursorAccount(target, { purgeCache: options.purgeCache });
  if (!removed.removed) {
    console.error(pc.red(`\n  Failed to log out: ${removed.error}\n`));
    process.exit(1);
  }

  if (options.purgeCache) {
    console.log(pc.green(`\n  Logged out from Cursor account (cache purged): ${pc.bold(target)}\n`));
  } else {
    console.log(pc.green(`\n  Logged out from Cursor account (history archived): ${pc.bold(target)}\n`));
  }
}

async function cursorStatus(options: { name?: string } = {}): Promise<void> {
  if (!isCursorLoggedIn()) {
    console.log(pc.yellow("\n  Not logged in to Cursor."));
    console.log(pc.gray("  Run 'tokscale cursor login' to authenticate.\n"));
    return;
  }

  const accounts = listCursorAccounts();
  const target = options.name
    ? options.name
    : accounts.find((a) => a.isActive)?.id;

  const credentials = target ? loadCursorCredentials(target) : null;
  if (!credentials) {
    console.log(pc.red("\n  Error: Cursor account not found."));
    console.log(pc.gray("  Run 'tokscale cursor accounts' to list saved accounts.\n"));
    process.exit(1);
  }

  console.log(pc.cyan("\n  Cursor IDE - Status\n"));
  if (accounts.length > 0) {
    console.log(pc.white("  Accounts:"));
    for (const acct of accounts) {
      const name = acct.label ? `${acct.label} ${pc.gray(`(${acct.id})`)}` : acct.id;
      console.log(`  ${acct.isActive ? pc.green("*") : pc.gray("-")} ${name}`);
    }
    console.log();
  }
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
      const usage = await readCursorUsage(target);
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
