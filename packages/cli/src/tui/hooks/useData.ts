import { createSignal, createEffect, on, type Accessor } from "solid-js";
import type {
  SourceType,
  SortType,
  ModelEntry,
  DailyEntry,
  ContributionDay,
  Stats,
  ModelWithPercentage,
  GridCell,
  TotalBreakdown,
  TUIData,
  ChartDataPoint,
} from "../types/index.js";
import {
  isNativeAvailable,
  parseLocalSourcesAsync,
  finalizeReportAsync,
  finalizeGraphAsync,
  type ParsedMessages,
} from "../../native.js";
import { PricingFetcher } from "../../pricing.js";
import { syncCursorCache, loadCursorCredentials } from "../../cursor.js";
import { getModelColor } from "../utils/colors.js";

export type {
  SortType,
  ModelEntry,
  DailyEntry,
  ContributionDay,
  Stats,
  ModelWithPercentage,
  GridCell,
  TotalBreakdown,
  TUIData,
};

export interface DateFilters {
  since?: string;
  until?: string;
  year?: string;
}

function buildContributionGrid(contributions: ContributionDay[]): GridCell[][] {
  const grid: GridCell[][] = Array.from({ length: 7 }, () => []);

  const today = new Date();
  const startYear = today.getFullYear();
  const startMonth = today.getMonth();
  const startDay = today.getDate() - 364;
  const startDate = new Date(startYear, startMonth, startDay);

  const contributionMap = new Map(contributions.map(c => [c.date, c.level]));

  const currentDate = new Date(startDate);
  while (currentDate <= today) {
    const year = currentDate.getFullYear();
    const month = String(currentDate.getMonth() + 1).padStart(2, "0");
    const day = String(currentDate.getDate()).padStart(2, "0");
    const dateStr = `${year}-${month}-${day}`;
    const dayOfWeek = currentDate.getDay();
    const level = contributionMap.get(dateStr) || 0;

    grid[dayOfWeek].push({ date: dateStr, level });
    currentDate.setDate(currentDate.getDate() + 1);
  }

  return grid;
}

function calculatePeakHour(messages: Array<{ timestamp: number }>): string {
  if (messages.length === 0) return "N/A";
  
  const hourCounts = new Array(24).fill(0);
  for (const msg of messages) {
    const hour = new Date(msg.timestamp).getHours();
    hourCounts[hour]++;
  }
  
  let maxCount = 0;
  let peakHour = 0;
  for (let h = 0; h < 24; h++) {
    if (hourCounts[h] > maxCount) {
      maxCount = hourCounts[h];
      peakHour = h;
    }
  }
  
  if (maxCount === 0) return "N/A";
  
  const suffix = peakHour >= 12 ? "pm" : "am";
  const displayHour = peakHour === 0 ? 12 : peakHour > 12 ? peakHour - 12 : peakHour;
  return `${displayHour}${suffix}`;
}

function calculateLongestSession(messages: Array<{ sessionId: string; timestamp: number }>): string {
  if (messages.length === 0) return "N/A";
  
  const sessions = new Map<string, number[]>();
  for (const msg of messages) {
    if (!msg.sessionId) continue;
    const timestamps = sessions.get(msg.sessionId) || [];
    timestamps.push(msg.timestamp);
    sessions.set(msg.sessionId, timestamps);
  }
  
  if (sessions.size === 0) return "N/A";
  
  let maxDurationMs = 0;
  for (const [, timestamps] of sessions) {
    if (timestamps.length < 2) continue;
    const minTs = Math.min(...timestamps);
    const maxTs = Math.max(...timestamps);
    const duration = maxTs - minTs;
    if (duration > maxDurationMs) {
      maxDurationMs = duration;
    }
  }
  
  if (maxDurationMs === 0) return "N/A";
  
  const totalSeconds = Math.floor(maxDurationMs / 1000);
  const hours = Math.floor(totalSeconds / 3600);
  const minutes = Math.floor((totalSeconds % 3600) / 60);
  
  if (hours > 0) {
    return `${hours}h ${minutes}m`;
  } else if (minutes > 0) {
    return `${minutes}m`;
  }
  return `${totalSeconds}s`;
}

async function loadData(enabledSources: Set<SourceType>, dateFilters?: DateFilters): Promise<TUIData> {
  if (!isNativeAvailable()) {
    throw new Error("Native module not available");
  }

  const sources = Array.from(enabledSources);
  const localSources = sources.filter(s => s !== "cursor");
  const includeCursor = sources.includes("cursor");
  const { since, until, year } = dateFilters ?? {};

  const pricingFetcher = new PricingFetcher();
  
  const [, cursorSync, localMessages] = await Promise.all([
    pricingFetcher.fetchPricing(),
    includeCursor && loadCursorCredentials() ? syncCursorCache() : Promise.resolve({ synced: false, rows: 0 }),
    localSources.length > 0
      ? parseLocalSourcesAsync({ sources: localSources as ("opencode" | "claude" | "codex" | "gemini")[], since, until, year })
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

  const [report, graph] = await Promise.all([
    finalizeReportAsync({
      localMessages: localMessages || emptyMessages,
      pricing: pricingFetcher.toPricingEntries(),
      includeCursor: includeCursor && cursorSync.synced,
      since,
      until,
      year,
    }),
    finalizeGraphAsync({
      localMessages: localMessages || emptyMessages,
      pricing: pricingFetcher.toPricingEntries(),
      includeCursor: includeCursor && cursorSync.synced,
      since,
      until,
      year,
    }),
  ]);

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

  let maxCost = 1;
  for (const d of dailyEntries) {
    if (d.cost > maxCost) maxCost = d.cost;
  }
  const contributions: ContributionDay[] = dailyEntries.map(d => ({
    date: d.date,
    cost: d.cost,
    level: d.cost === 0 ? 0 : (Math.min(4, Math.ceil((d.cost / maxCost) * 4)) as 0 | 1 | 2 | 3 | 4),
  }));

  const contributionGrid = buildContributionGrid(contributions);

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
  
  const sortedDates = dailyEntries.map(d => d.date).sort();
  if (sortedDates.length > 0) {
    const todayParts = new Date().toISOString().split("T")[0].split("-").map(Number);
    const todayUTC = Date.UTC(todayParts[0], todayParts[1] - 1, todayParts[2]);
    
    let streak = 0;
    for (let i = sortedDates.length - 1; i >= 0; i--) {
      const dateParts = sortedDates[i].split("-").map(Number);
      const dateUTC = Date.UTC(dateParts[0], dateParts[1] - 1, dateParts[2]);
      const daysFromToday = Math.round((todayUTC - dateUTC) / (1000 * 60 * 60 * 24));
      
      if (i === sortedDates.length - 1) {
        if (daysFromToday <= 1) {
          streak = 1;
        } else {
          break;
        }
      } else {
        const prevParts = sortedDates[i + 1].split("-").map(Number);
        const prevUTC = Date.UTC(prevParts[0], prevParts[1] - 1, prevParts[2]);
        const diffDays = Math.round((prevUTC - dateUTC) / (1000 * 60 * 60 * 24));
        if (diffDays === 1) {
          streak++;
        } else {
          break;
        }
      }
    }
    currentStreak = streak;
    
    streak = 1;
    for (let i = 1; i < sortedDates.length; i++) {
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
    longestStreak = Math.max(longestStreak, streak);
  }

  const stats: Stats = {
    favoriteModel,
    totalTokens: report.totalInput + report.totalOutput + report.totalCacheRead + report.totalCacheWrite,
    sessions: report.totalMessages,
    longestSession: calculateLongestSession(localMessages?.messages || []),
    currentStreak,
    longestStreak,
    activeDays: dailyEntries.length,
    totalDays: graph.summary.totalDays,
    peakHour: calculatePeakHour(localMessages?.messages || []),
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

  const totals: TotalBreakdown = {
    input: report.totalInput,
    output: report.totalOutput,
    cacheWrite: report.totalCacheWrite,
    cacheRead: report.totalCacheRead,
    total: report.totalInput + report.totalOutput + report.totalCacheWrite + report.totalCacheRead,
    cost: report.totalCost,
  };

  return {
    modelEntries,
    dailyEntries,
    contributions,
    contributionGrid,
    stats,
    totalCost: report.totalCost,
    totals,
    modelCount: modelEntries.length,
    chartData,
    topModels,
  };
}

export function useData(enabledSources: Accessor<Set<SourceType>>, dateFilters?: DateFilters) {
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
      loadData(sources, dateFilters)
        .then(setData)
        .catch((e) => setError(e.message))
        .finally(() => setLoading(false));
    }
  ));

  return { data, loading, error, refresh };
}
