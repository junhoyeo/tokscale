/**
 * Graph data generation module
 * Aggregates token usage data by date for contribution graph visualization
 *
 * Key design: intensity is calculated based on COST ($), not tokens
 */

import { format } from "date-fns";
import { readOpenCodeMessages } from "./opencode.js";
import {
  readClaudeCodeMessagesWithTimestamp,
  readCodexMessagesWithTimestamp,
} from "./claudecode.js";
import { readGeminiMessagesWithTimestamp } from "./gemini.js";
import { PricingFetcher } from "./pricing.js";
import type {
  TokenContributionData,
  DailyContribution,
  YearSummary,
  DataSummary,
  GraphOptions,
  UnifiedMessage,
  SourceType,
} from "./graph-types.js";

const VERSION = "1.0.0";

/**
 * Generate contribution graph data from all sources
 */
export async function generateGraphData(
  options: GraphOptions = {}
): Promise<TokenContributionData> {
  const fetcher = new PricingFetcher();
  await fetcher.fetchPricing();

  // Collect all messages from enabled sources
  const messages = collectMessages(options, fetcher);

  // Filter by date range
  const filteredMessages = filterMessagesByDate(messages, options);

  // Aggregate by date
  const dailyMap = aggregateByDate(filteredMessages, fetcher);

  // Convert to sorted array
  let contributions = Array.from(dailyMap.values()).sort((a, b) =>
    a.date.localeCompare(b.date)
  );

  // Calculate intensity based on COST
  contributions = calculateAllIntensities(contributions);

  // Build final structure
  const summary = calculateSummary(contributions);
  const years = calculateYears(contributions);

  const dateRange = {
    start: contributions[0]?.date ?? "",
    end: contributions[contributions.length - 1]?.date ?? "",
  };

  return {
    meta: {
      generatedAt: new Date().toISOString(),
      version: VERSION,
      dateRange,
    },
    summary,
    years,
    contributions,
  };
}

/**
 * Collect messages from all enabled sources
 */
function collectMessages(
  options: GraphOptions,
  fetcher: PricingFetcher
): UnifiedMessage[] {
  const messages: UnifiedMessage[] = [];
  const enabledSources = getEnabledSources(options);

  // OpenCode
  if (enabledSources.includes("opencode")) {
    const openCodeMessages = readOpenCodeMessages();
    for (const msg of openCodeMessages) {
      if (!msg.tokens || !msg.modelID) continue;

      const pricing = fetcher.getModelPricing(msg.modelID);
      const cost = pricing
        ? fetcher.calculateCost(
            {
              input: msg.tokens.input,
              output: msg.tokens.output,
              reasoning: msg.tokens.reasoning,
              cacheRead: msg.tokens.cache.read,
              cacheWrite: msg.tokens.cache.write,
            },
            pricing
          )
        : 0;

      messages.push({
        source: "opencode",
        modelId: msg.modelID,
        providerId: msg.providerID,
        timestamp: msg.time.created,
        tokens: {
          input: msg.tokens.input,
          output: msg.tokens.output,
          cacheRead: msg.tokens.cache.read,
          cacheWrite: msg.tokens.cache.write,
          reasoning: msg.tokens.reasoning,
        },
        cost,
      });
    }
  }

  // Claude Code
  if (enabledSources.includes("claude")) {
    const claudeMessages = readClaudeCodeMessagesWithTimestamp();
    for (const msg of claudeMessages) {
      const pricing = fetcher.getModelPricing(msg.model);
      const cost = pricing
        ? fetcher.calculateCost(
            {
              input: msg.tokens.input,
              output: msg.tokens.output,
              cacheRead: msg.tokens.cacheRead,
              cacheWrite: msg.tokens.cacheWrite,
            },
            pricing
          )
        : 0;

      messages.push({
        source: "claude",
        modelId: msg.model,
        providerId: "anthropic",
        timestamp: msg.timestamp,
        tokens: {
          input: msg.tokens.input,
          output: msg.tokens.output,
          cacheRead: msg.tokens.cacheRead,
          cacheWrite: msg.tokens.cacheWrite,
          reasoning: 0,
        },
        cost,
      });
    }
  }

  // Codex
  if (enabledSources.includes("codex")) {
    const codexMessages = readCodexMessagesWithTimestamp();
    for (const msg of codexMessages) {
      const pricing = fetcher.getModelPricing(msg.model);
      const cost = pricing
        ? fetcher.calculateCost(
            {
              input: msg.tokens.input,
              output: msg.tokens.output,
              cacheRead: msg.tokens.cacheRead,
              cacheWrite: msg.tokens.cacheWrite,
            },
            pricing
          )
        : 0;

      messages.push({
        source: "codex",
        modelId: msg.model,
        providerId: "openai",
        timestamp: msg.timestamp,
        tokens: {
          input: msg.tokens.input,
          output: msg.tokens.output,
          cacheRead: msg.tokens.cacheRead,
          cacheWrite: msg.tokens.cacheWrite,
          reasoning: 0,
        },
        cost,
      });
    }
  }

  // Gemini
  if (enabledSources.includes("gemini")) {
    const geminiMessages = readGeminiMessagesWithTimestamp();
    for (const msg of geminiMessages) {
      const pricing = fetcher.getModelPricing(msg.model);
      // Gemini: thoughts count as output for billing
      const cost = pricing
        ? fetcher.calculateCost(
            {
              input: msg.tokens.input,
              output: msg.tokens.output + msg.tokens.thoughts,
              cacheRead: 0, // Gemini cached tokens are free
              cacheWrite: 0,
            },
            pricing
          )
        : 0;

      messages.push({
        source: "gemini",
        modelId: msg.model,
        providerId: "google",
        timestamp: msg.timestamp,
        tokens: {
          input: msg.tokens.input,
          output: msg.tokens.output,
          cacheRead: msg.tokens.cached,
          cacheWrite: 0,
          reasoning: msg.tokens.thoughts,
        },
        cost,
      });
    }
  }

  return messages;
}

/**
 * Get list of enabled sources based on options
 */
function getEnabledSources(options: GraphOptions): SourceType[] {
  if (options.sources && options.sources.length > 0) {
    return options.sources;
  }
  return ["opencode", "claude", "codex", "gemini"];
}

/**
 * Filter messages by date range
 */
function filterMessagesByDate(
  messages: UnifiedMessage[],
  options: GraphOptions
): UnifiedMessage[] {
  let filtered = messages;

  // Filter by year
  if (options.year) {
    const yearStart = new Date(`${options.year}-01-01`).getTime();
    const yearEnd = new Date(`${options.year}-12-31T23:59:59.999`).getTime();
    filtered = filtered.filter(
      (m) => m.timestamp >= yearStart && m.timestamp <= yearEnd
    );
  }

  // Filter by since
  if (options.since) {
    const sinceTime = new Date(options.since).getTime();
    filtered = filtered.filter((m) => m.timestamp >= sinceTime);
  }

  // Filter by until
  if (options.until) {
    const untilTime = new Date(`${options.until}T23:59:59.999`).getTime();
    filtered = filtered.filter((m) => m.timestamp <= untilTime);
  }

  return filtered;
}

/**
 * Aggregate messages by date
 */
function aggregateByDate(
  messages: UnifiedMessage[],
  _fetcher: PricingFetcher
): Map<string, DailyContribution> {
  const dailyMap = new Map<string, DailyContribution>();

  for (const msg of messages) {
    const dateKey = format(new Date(msg.timestamp), "yyyy-MM-dd");

    let daily = dailyMap.get(dateKey);
    if (!daily) {
      daily = createEmptyDailyContribution(dateKey);
      dailyMap.set(dateKey, daily);
    }

    // Update totals
    const totalTokens =
      msg.tokens.input +
      msg.tokens.output +
      msg.tokens.cacheRead +
      msg.tokens.cacheWrite +
      msg.tokens.reasoning;

    daily.totals.tokens += totalTokens;
    daily.totals.cost += msg.cost;
    daily.totals.messages += 1;

    // Update token breakdown
    daily.tokenBreakdown.input += msg.tokens.input;
    daily.tokenBreakdown.output += msg.tokens.output;
    daily.tokenBreakdown.cacheRead += msg.tokens.cacheRead;
    daily.tokenBreakdown.cacheWrite += msg.tokens.cacheWrite;
    daily.tokenBreakdown.reasoning += msg.tokens.reasoning;

    // Update source contributions
    let sourceContrib = daily.sources.find(
      (s) => s.source === msg.source && s.modelId === msg.modelId
    );

    if (!sourceContrib) {
      sourceContrib = {
        source: msg.source,
        modelId: msg.modelId,
        providerId: msg.providerId,
        tokens: {
          input: 0,
          output: 0,
          cacheRead: 0,
          cacheWrite: 0,
          reasoning: 0,
        },
        cost: 0,
        messages: 0,
      };
      daily.sources.push(sourceContrib);
    }

    sourceContrib.tokens.input += msg.tokens.input;
    sourceContrib.tokens.output += msg.tokens.output;
    sourceContrib.tokens.cacheRead += msg.tokens.cacheRead;
    sourceContrib.tokens.cacheWrite += msg.tokens.cacheWrite;
    sourceContrib.tokens.reasoning += msg.tokens.reasoning;
    sourceContrib.cost += msg.cost;
    sourceContrib.messages += 1;
  }

  return dailyMap;
}

/**
 * Create empty daily contribution
 */
function createEmptyDailyContribution(date: string): DailyContribution {
  return {
    date,
    totals: {
      tokens: 0,
      cost: 0,
      messages: 0,
    },
    intensity: 0,
    tokenBreakdown: {
      input: 0,
      output: 0,
      cacheRead: 0,
      cacheWrite: 0,
      reasoning: 0,
    },
    sources: [],
  };
}

/**
 * Calculate intensity for all contributions based on COST
 */
function calculateAllIntensities(
  contributions: DailyContribution[]
): DailyContribution[] {
  if (contributions.length === 0) return contributions;

  // Find max cost for intensity calculation
  const maxCost = Math.max(...contributions.map((c) => c.totals.cost));

  return contributions.map((c) => ({
    ...c,
    intensity: calculateIntensity(c.totals.cost, maxCost),
  }));
}

/**
 * Calculate intensity grade based on cost ratio
 * 0 = no activity, 4 = highest
 */
function calculateIntensity(
  cost: number,
  maxCost: number
): 0 | 1 | 2 | 3 | 4 {
  if (cost === 0 || maxCost === 0) return 0;
  const ratio = cost / maxCost;
  if (ratio >= 0.75) return 4;
  if (ratio >= 0.5) return 3;
  if (ratio >= 0.25) return 2;
  return 1;
}

/**
 * Calculate summary statistics
 */
function calculateSummary(contributions: DailyContribution[]): DataSummary {
  const totalTokens = contributions.reduce((sum, c) => sum + c.totals.tokens, 0);
  const totalCost = contributions.reduce((sum, c) => sum + c.totals.cost, 0);
  const activeDays = contributions.filter((c) => c.totals.cost > 0).length;
  const maxCostInSingleDay = Math.max(
    ...contributions.map((c) => c.totals.cost),
    0
  );

  // Collect unique sources and models
  const sourcesSet = new Set<SourceType>();
  const modelsSet = new Set<string>();

  for (const c of contributions) {
    for (const s of c.sources) {
      sourcesSet.add(s.source);
      modelsSet.add(s.modelId);
    }
  }

  return {
    totalTokens,
    totalCost,
    totalDays: contributions.length,
    activeDays,
    averagePerDay: activeDays > 0 ? totalCost / activeDays : 0,
    maxCostInSingleDay,
    sources: Array.from(sourcesSet),
    models: Array.from(modelsSet),
  };
}

/**
 * Calculate year summaries
 */
function calculateYears(contributions: DailyContribution[]): YearSummary[] {
  const yearsMap = new Map<
    string,
    { tokens: number; cost: number; start: string; end: string }
  >();

  for (const c of contributions) {
    const year = c.date.substring(0, 4);
    let yearData = yearsMap.get(year);

    if (!yearData) {
      yearData = { tokens: 0, cost: 0, start: c.date, end: c.date };
      yearsMap.set(year, yearData);
    }

    yearData.tokens += c.totals.tokens;
    yearData.cost += c.totals.cost;
    if (c.date < yearData.start) yearData.start = c.date;
    if (c.date > yearData.end) yearData.end = c.date;
  }

  return Array.from(yearsMap.entries())
    .map(([year, data]) => ({
      year,
      totalTokens: data.tokens,
      totalCost: data.cost,
      range: {
        start: data.start,
        end: data.end,
      },
    }))
    .sort((a, b) => a.year.localeCompare(b.year));
}
