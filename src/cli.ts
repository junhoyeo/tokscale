#!/usr/bin/env node
/**
 * Token Tracker CLI
 * Display OpenCode, Claude Code, Codex, and Gemini usage with dynamic width tables
 * 
 * All heavy computation is done in the native Rust module.
 */

import { Command } from "commander";
import pc from "picocolors";
import { login, logout, whoami } from "./auth.js";
import { submit } from "./submit.js";
import { PricingFetcher } from "./pricing.js";
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
  getModelReportNative,
  getMonthlyReportNative,
  generateGraphWithPricing,
  type ModelReport,
  type MonthlyReport,
} from "./native.js";
import * as fs from "node:fs";
import { performance } from "node:perf_hooks";
import type { SourceType } from "./graph-types.js";

interface FilterOptions {
  opencode?: boolean;
  claude?: boolean;
  codex?: boolean;
  gemini?: boolean;
}

async function main() {
  const program = new Command();

  program
    .name("token-tracker")
    .description("Calculate token prices from OpenCode, Claude Code, Codex, and Gemini sessions")
    .version("1.0.0");

  program
    .command("monthly")
    .description("Show monthly usage report")
    .option("--opencode", "Show only OpenCode usage")
    .option("--claude", "Show only Claude Code usage")
    .option("--codex", "Show only Codex CLI usage")
    .option("--gemini", "Show only Gemini CLI usage")
    .option("--benchmark", "Show processing time")
    .action(async (options) => {
      await showMonthlyReport(options);
    });

  program
    .command("models")
    .description("Show usage breakdown by model")
    .option("--opencode", "Show only OpenCode usage")
    .option("--claude", "Show only Claude Code usage")
    .option("--codex", "Show only Codex CLI usage")
    .option("--gemini", "Show only Gemini CLI usage")
    .option("--benchmark", "Show processing time")
    .action(async (options) => {
      await showModelReport(options);
    });

  program
    .command("graph")
    .description("Export contribution graph data as JSON")
    .option("--output <file>", "Write to file instead of stdout")
    .option("--opencode", "Include only OpenCode data")
    .option("--claude", "Include only Claude Code data")
    .option("--codex", "Include only Codex CLI data")
    .option("--gemini", "Include only Gemini CLI data")
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
    .description("Login to Token Tracker (opens browser for GitHub auth)")
    .action(async () => {
      await login();
    });

  program
    .command("logout")
    .description("Logout from Token Tracker")
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
    .description("Submit your usage data to Token Tracker")
    .option("--opencode", "Include only OpenCode data")
    .option("--claude", "Include only Claude Code data")
    .option("--codex", "Include only Codex CLI data")
    .option("--gemini", "Include only Gemini CLI data")
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
        since: options.since,
        until: options.until,
        year: options.year,
        dryRun: options.dryRun,
      });
    });

  // Default command with options
  program
    .option("--opencode", "Show only OpenCode usage")
    .option("--claude", "Show only Claude Code usage")
    .option("--codex", "Show only Codex CLI usage")
    .option("--gemini", "Show only Gemini CLI usage")
    .option("--benchmark", "Show processing time")
    .action(async (options) => {
      await showModelReport(options);
    });

  await program.parseAsync();
}

function getEnabledSources(options: FilterOptions): SourceType[] | undefined {
  const hasFilter = options.opencode || options.claude || options.codex || options.gemini;
  if (!hasFilter) return undefined; // All sources

  const sources: SourceType[] = [];
  if (options.opencode) sources.push("opencode");
  if (options.claude) sources.push("claude");
  if (options.codex) sources.push("codex");
  if (options.gemini) sources.push("gemini");
  return sources;
}

async function ensureNativeModule(): Promise<void> {
  if (!isNativeAvailable()) {
    console.error(pc.red("Error: Native Rust module not available."));
    console.error(pc.gray("Run 'yarn build:core' to build the native module."));
    process.exit(1);
  }
}

async function fetchPricingWithCache(): Promise<PricingFetcher> {
  const fetcher = new PricingFetcher();
  console.log(pc.gray("  Fetching pricing data..."));
  await fetcher.fetchPricing();
  return fetcher;
}

async function showModelReport(options: FilterOptions & { benchmark?: boolean }) {
  await ensureNativeModule();

  console.log(pc.cyan("\n  Token Usage Report by Model"));
  if (options.benchmark) {
    console.log(pc.gray(`  Using: Rust native module v${getNativeVersion()}`));
  }
  console.log();

  const fetcher = await fetchPricingWithCache();
  const pricingEntries = fetcher.toPricingEntries();

  const startTime = performance.now();

  let report: ModelReport;
  try {
    report = getModelReportNative({
      sources: getEnabledSources(options),
      pricing: pricingEntries,
    });
  } catch (e) {
    console.error(pc.red(`Error: ${(e as Error).message}`));
    process.exit(1);
  }

  const processingTime = performance.now() - startTime;

  if (report.entries.length === 0) {
    console.log(pc.yellow("  No usage data found.\n"));
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
  }

  console.log();
}

async function showMonthlyReport(options: FilterOptions & { benchmark?: boolean }) {
  await ensureNativeModule();

  console.log(pc.cyan("\n  Monthly Token Usage Report"));
  if (options.benchmark) {
    console.log(pc.gray(`  Using: Rust native module v${getNativeVersion()}`));
  }
  console.log();

  const fetcher = await fetchPricingWithCache();
  const pricingEntries = fetcher.toPricingEntries();

  const startTime = performance.now();

  let report: MonthlyReport;
  try {
    report = getMonthlyReportNative({
      sources: getEnabledSources(options),
      pricing: pricingEntries,
    });
  } catch (e) {
    console.error(pc.red(`Error: ${(e as Error).message}`));
    process.exit(1);
  }

  const processingTime = performance.now() - startTime;

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
  }

  console.log();
}

interface GraphCommandOptions {
  output?: string;
  opencode?: boolean;
  claude?: boolean;
  codex?: boolean;
  gemini?: boolean;
  since?: string;
  until?: string;
  year?: string;
  benchmark?: boolean;
}

async function handleGraphCommand(options: GraphCommandOptions) {
  await ensureNativeModule();

  const fetcher = await fetchPricingWithCache();
  const pricingEntries = fetcher.toPricingEntries();

  const startTime = performance.now();

  // Determine which sources to include
  const sources = getEnabledSources(options);

  // Generate graph data using native module
  const data = generateGraphWithPricing({
    sources,
    pricing: pricingEntries,
    since: options.since,
    until: options.until,
    year: options.year,
  });

  const processingTime = performance.now() - startTime;

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
    default:
      return source;
  }
}

main().catch(console.error);
