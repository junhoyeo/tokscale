#!/usr/bin/env node
/**
 * Token Tracker CLI
 * Display OpenCode, Claude Code, Codex, and Gemini usage with dynamic width tables
 */

import { Command } from "commander";
import pc from "picocolors";
import { PricingFetcher } from "./pricing.js";
import {
  createUsageTable,
  formatUsageRow,
  formatTotalsRow,
  formatNumber,
  formatCurrency,
  formatModelName,
} from "./table.js";
import { readOpenCodeMessages, aggregateOpenCodeByModel } from "./opencode.js";
import { readClaudeCodeSessions, readCodexSessions } from "./claudecode.js";
import { readGeminiSessions } from "./gemini.js";
import { generateGraphData } from "./graph.js";
import * as fs from "node:fs";
import type { SourceType } from "./graph-types.js";

interface UsageSummary {
  source: string;
  model: string;
  input: number;
  output: number;
  cacheRead: number;
  cacheWrite: number;
  reasoning: number;
  messageCount: number;
  cost: number;
}

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
    .action(async (options) => {
      await handleGraphCommand(options);
    });

  // Default command with options
  program
    .option("--opencode", "Show only OpenCode usage")
    .option("--claude", "Show only Claude Code usage")
    .option("--codex", "Show only Codex CLI usage")
    .option("--gemini", "Show only Gemini CLI usage")
    .action(async (options) => {
      await showModelReport(options);
    });

  await program.parseAsync();
}

function shouldInclude(
  options: FilterOptions,
  source: "opencode" | "claude" | "codex" | "gemini"
): boolean {
  const hasFilter = options.opencode || options.claude || options.codex || options.gemini;
  if (!hasFilter) return true; // No filter = include all
  return !!options[source];
}

async function showModelReport(options: FilterOptions) {
  console.log(pc.cyan("\n  Token Usage Report by Model\n"));

  const fetcher = new PricingFetcher();
  console.log(pc.gray("  Fetching pricing data from LiteLLM..."));
  await fetcher.fetchPricing();

  const summaries: UsageSummary[] = [];

  // OpenCode data (~/.local/share/opencode/)
  if (shouldInclude(options, "opencode")) {
    const openCodeMessages = readOpenCodeMessages();
    if (openCodeMessages.length > 0) {
      const openCodeData = aggregateOpenCodeByModel(openCodeMessages);

      for (const data of openCodeData.values()) {
        const pricing = fetcher.getModelPricing(data.modelID);
        const cost = pricing
          ? fetcher.calculateCost(
              {
                input: data.input,
                output: data.output,
                reasoning: data.reasoning,
                cacheRead: data.cacheRead,
                cacheWrite: data.cacheWrite,
              },
              pricing
            )
          : 0;

        summaries.push({
          source: "OpenCode",
          model: data.modelID,
          input: data.input,
          output: data.output,
          cacheRead: data.cacheRead,
          cacheWrite: data.cacheWrite,
          reasoning: data.reasoning,
          messageCount: data.messageCount,
          cost,
        });
      }
    }
  }

  // Claude Code data (~/.claude/projects/)
  if (shouldInclude(options, "claude")) {
    const claudeCodeData = readClaudeCodeSessions();
    if (claudeCodeData.length > 0) {
      for (const data of claudeCodeData) {
        const pricing = fetcher.getModelPricing(data.model);
        const cost = pricing
          ? fetcher.calculateCost(
              {
                input: data.input,
                output: data.output,
                cacheRead: data.cacheRead,
                cacheWrite: data.cacheWrite,
              },
              pricing
            )
          : 0;

        summaries.push({
          source: "Claude",
          model: data.model,
          input: data.input,
          output: data.output,
          cacheRead: data.cacheRead,
          cacheWrite: data.cacheWrite,
          reasoning: 0,
          messageCount: data.messageCount,
          cost,
        });
      }
    }
  }

  // Codex CLI data (~/.codex/sessions/)
  if (shouldInclude(options, "codex")) {
    const codexData = readCodexSessions();
    if (codexData.length > 0) {
      for (const data of codexData) {
        const pricing = fetcher.getModelPricing(data.model);
        const cost = pricing
          ? fetcher.calculateCost(
              {
                input: data.input,
                output: data.output,
                cacheRead: data.cacheRead,
                cacheWrite: data.cacheWrite,
              },
              pricing
            )
          : 0;

        summaries.push({
          source: "Codex",
          model: data.model,
          input: data.input,
          output: data.output,
          cacheRead: data.cacheRead,
          cacheWrite: data.cacheWrite,
          reasoning: 0,
          messageCount: data.messageCount,
          cost,
        });
      }
    }
  }

  // Gemini CLI data (~/.gemini/tmp/)
  if (shouldInclude(options, "gemini")) {
    const geminiData = readGeminiSessions();
    if (geminiData.length > 0) {
      for (const data of geminiData) {
        const pricing = fetcher.getModelPricing(data.model);
        // Gemini: cached tokens are free, thoughts count as output
        const cost = pricing
          ? fetcher.calculateCost(
              {
                input: data.input,
                output: data.output + data.thoughts, // thoughts charged as output
                cacheRead: 0, // cached tokens are free for Gemini
                cacheWrite: 0,
              },
              pricing
            )
          : 0;

        summaries.push({
          source: "Gemini",
          model: data.model,
          input: data.input,
          output: data.output + data.thoughts,
          cacheRead: data.cached, // Display cached but don't charge
          cacheWrite: 0,
          reasoning: data.thoughts,
          messageCount: data.messageCount,
          cost,
        });
      }
    }
  }

  if (summaries.length === 0) {
    console.log(pc.yellow("  No usage data found.\n"));
    return;
  }

  // Sort by cost descending
  summaries.sort((a, b) => b.cost - a.cost);

  // Create table
  const table = createUsageTable("Source/Model");

  let totalInput = 0;
  let totalOutput = 0;
  let totalCacheRead = 0;
  let totalCacheWrite = 0;
  let totalCost = 0;

  for (const summary of summaries) {
    const modelDisplay = `${pc.dim(summary.source)} ${formatModelName(summary.model)}`;
    table.push(
      formatUsageRow(
        modelDisplay,
        [summary.model],
        summary.input,
        summary.output,
        summary.cacheWrite,
        summary.cacheRead,
        summary.cost
      )
    );

    totalInput += summary.input;
    totalOutput += summary.output;
    totalCacheRead += summary.cacheRead;
    totalCacheWrite += summary.cacheWrite;
    totalCost += summary.cost;
  }

  // Add totals row
  table.push(formatTotalsRow(totalInput, totalOutput, totalCacheWrite, totalCacheRead, totalCost));

  console.log(table.toString());

  // Summary stats
  console.log(
    pc.gray(
      `\n  Total: ${formatNumber(summaries.reduce((sum, s) => sum + s.messageCount, 0))} messages, ` +
        `${formatNumber(totalInput + totalOutput + totalCacheRead + totalCacheWrite)} tokens, ` +
        `${pc.green(formatCurrency(totalCost))}\n`
    )
  );
}

async function showMonthlyReport(options: FilterOptions) {
  console.log(pc.cyan("\n  Monthly Token Usage Report\n"));

  const fetcher = new PricingFetcher();
  console.log(pc.gray("  Fetching pricing data from LiteLLM..."));
  await fetcher.fetchPricing();

  // Group by month
  const monthlyData = new Map<
    string,
    {
      models: Set<string>;
      input: number;
      output: number;
      cacheRead: number;
      cacheWrite: number;
      cost: number;
    }
  >();

  // OpenCode data
  if (shouldInclude(options, "opencode")) {
    const openCodeMessages = readOpenCodeMessages();
    for (const msg of openCodeMessages) {
      if (!msg.tokens || !msg.modelID) continue;

      const date = new Date(msg.time.created);
      const monthKey = `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}`;

      let monthly = monthlyData.get(monthKey);
      if (!monthly) {
        monthly = {
          models: new Set(),
          input: 0,
          output: 0,
          cacheRead: 0,
          cacheWrite: 0,
          cost: 0,
        };
        monthlyData.set(monthKey, monthly);
      }

      monthly.models.add(msg.modelID);
      monthly.input += msg.tokens.input;
      monthly.output += msg.tokens.output;
      monthly.cacheRead += msg.tokens.cache.read;
      monthly.cacheWrite += msg.tokens.cache.write;

      const pricing = fetcher.getModelPricing(msg.modelID);
      if (pricing) {
        monthly.cost += fetcher.calculateCost(
          {
            input: msg.tokens.input,
            output: msg.tokens.output,
            reasoning: msg.tokens.reasoning,
            cacheRead: msg.tokens.cache.read,
            cacheWrite: msg.tokens.cache.write,
          },
          pricing
        );
      }
    }
  }

  // TODO: Add Claude Code, Codex, and Gemini monthly aggregation
  // (would need timestamp data from those sources)

  if (monthlyData.size === 0) {
    console.log(pc.yellow("  No usage data found.\n"));
    return;
  }

  // Sort by month
  const sortedMonths = Array.from(monthlyData.entries()).sort((a, b) => a[0].localeCompare(b[0]));

  const table = createUsageTable("Month");

  let totalInput = 0;
  let totalOutput = 0;
  let totalCacheRead = 0;
  let totalCacheWrite = 0;
  let totalCost = 0;

  for (const [month, data] of sortedMonths) {
    table.push(
      formatUsageRow(
        month,
        Array.from(data.models),
        data.input,
        data.output,
        data.cacheWrite,
        data.cacheRead,
        data.cost
      )
    );

    totalInput += data.input;
    totalOutput += data.output;
    totalCacheRead += data.cacheRead;
    totalCacheWrite += data.cacheWrite;
    totalCost += data.cost;
  }

  table.push(formatTotalsRow(totalInput, totalOutput, totalCacheWrite, totalCacheRead, totalCost));

  console.log(table.toString());
  console.log(pc.gray(`\n  Total Cost: ${pc.green(formatCurrency(totalCost))}\n`));
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
}

async function handleGraphCommand(options: GraphCommandOptions) {
  // Determine which sources to include
  const sources: SourceType[] = [];
  const hasSourceFilter = options.opencode || options.claude || options.codex || options.gemini;

  if (!hasSourceFilter) {
    sources.push("opencode", "claude", "codex", "gemini");
  } else {
    if (options.opencode) sources.push("opencode");
    if (options.claude) sources.push("claude");
    if (options.codex) sources.push("codex");
    if (options.gemini) sources.push("gemini");
  }

  // Generate graph data
  const data = await generateGraphData({
    sources,
    since: options.since,
    until: options.until,
    year: options.year,
  });

  const jsonOutput = JSON.stringify(data, null, 2);

  // Output to file or stdout
  if (options.output) {
    fs.writeFileSync(options.output, jsonOutput, "utf-8");
    console.error(pc.green(`✓ Graph data written to ${options.output}`));
    console.error(pc.gray(`  ${data.contributions.length} days, ${data.summary.sources.length} sources, ${data.summary.models.length} models`));
    console.error(pc.gray(`  Total: ${formatCurrency(data.summary.totalCost)}`));
  } else {
    console.log(jsonOutput);
  }
}

main().catch(console.error);
