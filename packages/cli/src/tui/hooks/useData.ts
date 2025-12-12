import { createSignal, createEffect, on, type Accessor } from "solid-js";
import type { SourceType } from "../App.js";
import {
  isNativeAvailable,
  parseLocalSourcesNative,
  finalizeReportNative,
  finalizeGraphNative,
  type ParsedMessages,
} from "../../native.js";
import { PricingFetcher } from "../../pricing.js";
import { syncCursorCache, loadCursorCredentials } from "../../cursor.js";
import { getModelColor } from "../utils/colors.js";
import type { ChartDataPoint } from "../components/BarChart.js";

export type { SortType } from "../App.js";

export interface ModelEntry {
  source: string;
  model: string;
  input: number;
  output: number;
  cacheWrite: number;
  cacheRead: number;
  total: number;
  cost: number;
}

export interface DailyEntry {
  date: string;
  input: number;
  output: number;
  cache: number;
  total: number;
  cost: number;
}

export interface ContributionDay {
  date: string;
  cost: number;
  level: number;
}

export interface Stats {
  favoriteModel: string;
  totalTokens: number;
  sessions: number;
  longestSession: string;
  currentStreak: number;
  longestStreak: number;
  activeDays: number;
  totalDays: number;
  peakHour: string;
}

export interface ModelWithPercentage {
  modelId: string;
  percentage: number;
  inputTokens: number;
  outputTokens: number;
  totalTokens: number;
  cost: number;
}

export interface TUIData {
  modelEntries: ModelEntry[];
  dailyEntries: DailyEntry[];
  contributions: ContributionDay[];
  stats: Stats;
  totalCost: number;
  modelCount: number;
  chartData: ChartDataPoint[];
  topModels: ModelWithPercentage[];
}

async function loadData(enabledSources: Set<SourceType>): Promise<TUIData> {
  if (!isNativeAvailable()) {
    throw new Error("Native module not available");
  }

  const sources = Array.from(enabledSources);
  const localSources = sources.filter(s => s !== "cursor");
  const includeCursor = sources.includes("cursor");

  const pricingFetcher = new PricingFetcher();
  
  const [, cursorSync, localMessages] = await Promise.all([
    pricingFetcher.fetchPricing(),
    includeCursor && loadCursorCredentials() ? syncCursorCache() : Promise.resolve({ synced: false, rows: 0 }),
    localSources.length > 0
      ? parseLocalSourcesNative({ sources: localSources as ("opencode" | "claude" | "codex" | "gemini")[] })
      : Promise.resolve({ messages: [], opencodeCount: 0, claudeCount: 0, codexCount: 0, geminiCount: 0, processingTimeMs: 0 } as ParsedMessages),
  ]);

  const emptyMessages: ParsedMessages = {
    messages: [],
    opencodeCount: 0,
    claudeCount: 0,
    codexCount: 0,
    geminiCount: 0,
    processingTimeMs: 0,
  };

  const report = finalizeReportNative({
    localMessages: localMessages || emptyMessages,
    pricing: pricingFetcher.toPricingEntries(),
    includeCursor: includeCursor && cursorSync.synced,
  });

  const graph = finalizeGraphNative({
    localMessages: localMessages || emptyMessages,
    pricing: pricingFetcher.toPricingEntries(),
    includeCursor: includeCursor && cursorSync.synced,
  });

  const modelEntries: ModelEntry[] = report.entries.map(e => ({
    source: e.source,
    model: e.model,
    input: e.input,
    output: e.output,
    cacheWrite: e.cacheWrite,
    cacheRead: e.cacheRead,
    total: e.input + e.output + e.cacheWrite + e.cacheRead,
    cost: e.cost,
  }));

  const dailyMap = new Map<string, DailyEntry>();
  for (const contrib of graph.contributions) {
    const dateStr = contrib.date;
    if (!dailyMap.has(dateStr)) {
      dailyMap.set(dateStr, {
        date: dateStr,
        input: 0,
        output: 0,
        cache: 0,
        total: 0,
        cost: 0,
      });
    }
    const entry = dailyMap.get(dateStr)!;
    entry.input += contrib.tokenBreakdown.input;
    entry.output += contrib.tokenBreakdown.output;
    entry.cache += contrib.tokenBreakdown.cacheRead;
    entry.total += contrib.totals.tokens;
    entry.cost += contrib.totals.cost;
  }
  const dailyEntries = Array.from(dailyMap.values()).sort((a, b) => b.date.localeCompare(a.date));

  const maxCost = Math.max(...dailyEntries.map(d => d.cost), 1);
  const contributions: ContributionDay[] = dailyEntries.map(d => ({
    date: d.date,
    cost: d.cost,
    level: Math.min(4, Math.floor((d.cost / maxCost) * 5)),
  }));

  const modelCosts = new Map<string, number>();
  for (const e of modelEntries) {
    const current = modelCosts.get(e.model) || 0;
    modelCosts.set(e.model, current + e.cost);
  }
  let favoriteModel = "N/A";
  let maxModelCost = 0;
  for (const [model, cost] of modelCosts) {
    if (cost > maxModelCost) {
      maxModelCost = cost;
      favoriteModel = model;
    }
  }

  let currentStreak = 0;
  let longestStreak = 0;
  let streak = 0;
  const sortedDates = dailyEntries.map(d => d.date).sort();
  for (let i = 0; i < sortedDates.length; i++) {
    if (i === 0) {
      streak = 1;
    } else {
      const prevParts = sortedDates[i - 1].split("-").map(Number);
      const currParts = sortedDates[i].split("-").map(Number);
      const prevDate = Date.UTC(prevParts[0], prevParts[1] - 1, prevParts[2]);
      const currDate = Date.UTC(currParts[0], currParts[1] - 1, currParts[2]);
      const diffDays = Math.round((currDate - prevDate) / (1000 * 60 * 60 * 24));
      if (diffDays === 1) {
        streak++;
      } else {
        longestStreak = Math.max(longestStreak, streak);
        streak = 1;
      }
    }
  }
  longestStreak = Math.max(longestStreak, streak);
  currentStreak = streak;

  const stats: Stats = {
    favoriteModel,
    totalTokens: report.totalInput + report.totalOutput + report.totalCacheRead + report.totalCacheWrite,
    sessions: report.totalMessages,
    longestSession: "N/A",
    currentStreak,
    longestStreak,
    activeDays: dailyEntries.length,
    totalDays: 365,
    peakHour: "N/A",
  };

  const dailyModelMap = new Map<string, Map<string, number>>();
  for (const contrib of graph.contributions) {
    const dateStr = contrib.date;
    if (!dailyModelMap.has(dateStr)) {
      dailyModelMap.set(dateStr, new Map());
    }
    const modelMap = dailyModelMap.get(dateStr)!;
    for (const source of contrib.sources) {
      const modelId = source.modelId;
      const tokens = source.tokens.input + source.tokens.output + source.tokens.cacheRead;
      modelMap.set(modelId, (modelMap.get(modelId) || 0) + tokens);
    }
  }

  const chartData: ChartDataPoint[] = Array.from(dailyModelMap.entries())
    .map(([date, modelMap]) => {
      const models = Array.from(modelMap.entries()).map(([modelId, tokens]) => ({
        modelId,
        tokens,
        color: getModelColor(modelId),
      }));
      return {
        date,
        models,
        total: models.reduce((sum, m) => sum + m.tokens, 0),
      };
    })
    .sort((a, b) => a.date.localeCompare(b.date));

  const modelTokensMap = new Map<string, { input: number; output: number; cost: number }>();
  for (const e of modelEntries) {
    const existing = modelTokensMap.get(e.model) || { input: 0, output: 0, cost: 0 };
    modelTokensMap.set(e.model, {
      input: existing.input + e.input,
      output: existing.output + e.output,
      cost: existing.cost + e.cost,
    });
  }

  const totalTokensSum = stats.totalTokens || 1;
  const topModels: ModelWithPercentage[] = Array.from(modelTokensMap.entries())
    .map(([modelId, data]) => {
      const totalTokens = data.input + data.output;
      return {
        modelId,
        percentage: (totalTokens / totalTokensSum) * 100,
        inputTokens: data.input,
        outputTokens: data.output,
        totalTokens,
        cost: data.cost,
      };
    })
    .sort((a, b) => b.cost - a.cost);

  return {
    modelEntries,
    dailyEntries,
    contributions,
    stats,
    totalCost: report.totalCost,
    modelCount: modelEntries.length,
    chartData,
    topModels,
  };
}

export function useData(enabledSources: Accessor<Set<SourceType>>) {
  const [data, setData] = createSignal<TUIData | null>(null);
  const [loading, setLoading] = createSignal(true);
  const [error, setError] = createSignal<string | null>(null);
  const [refreshTrigger, setRefreshTrigger] = createSignal(0);

  const refresh = () => {
    setRefreshTrigger(prev => prev + 1);
  };

  createEffect(on(
    () => [enabledSources(), refreshTrigger()] as const,
    ([sources]) => {
      setLoading(true);
      setError(null);
      loadData(sources)
        .then(setData)
        .catch((e) => setError(e.message))
        .finally(() => setLoading(false));
    }
  ));

  return { data, loading, error, refresh };
}
