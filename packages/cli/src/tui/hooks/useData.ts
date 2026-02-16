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
  LoadingPhase,
  DailyModelBreakdown,
} from "../types/index.js";
import {
  parseLocalSourcesAsync,
  finalizeReportAndGraphAsync,
  type ParsedMessages,
} from "../../native.js";

import { syncCursorCache, isCursorLoggedIn, hasCursorUsageCache } from "../../cursor.js";
import { getModelColor } from "../utils/colors.js";
import { loadCachedData, saveCachedData, isCacheStale, loadSettings } from "../config/settings.js";
import { formatDateLocal } from "../../date-utils.js";

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
  LoadingPhase,
};

export interface DateFilters {
  since?: string;
  until?: string;
  year?: string;
  sinceTs?: number;
  untilTs?: number;
}

function buildContributionGrid(contributions: ContributionDay[]): GridCell[][] {
  const grid: GridCell[][] = Array.from({ length: 7 }, () => []);

  const today = new Date();
  const todayStr = formatDateLocal(today);
  
  const startDate = new Date(today);
  startDate.setDate(startDate.getDate() - 364);
  while (startDate.getDay() !== 0) {
    startDate.setDate(startDate.getDate() - 1);
  }

  const endDate = new Date(today);
  while (endDate.getDay() !== 6) {
    endDate.setDate(endDate.getDate() + 1);
  }

  const contributionMap = new Map(contributions.map(c => [c.date, c.level]));

  const currentDate = new Date(startDate);
  while (currentDate <= endDate) {
    const dateStr = formatDateLocal(currentDate);
    const dayOfWeek = currentDate.getDay();
    
    const isFuture = dateStr > todayStr;
    const level = isFuture ? 0 : (contributionMap.get(dateStr) || 0);

    grid[dayOfWeek].push({ date: isFuture ? null : dateStr, level });
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

async function loadData(
  enabledSources: Set<SourceType>, 
  dateFilters?: DateFilters,
  setPhase?: (phase: LoadingPhase) => void
): Promise<TUIData> {
  const sources = Array.from(enabledSources);
  const localSources = sources.filter(s => s !== "cursor");
  const includeCursor = sources.includes("cursor");
  const { since, until, year, sinceTs, untilTs } = dateFilters ?? {};

  setPhase?.("parsing-sources");
  
  const phase1Results = await Promise.allSettled([
    includeCursor && isCursorLoggedIn() ? syncCursorCache() : Promise.resolve({ synced: false, rows: 0, error: undefined }),
    localSources.length > 0
      ? parseLocalSourcesAsync({ sources: localSources as ("opencode" | "claude" | "codex" | "gemini" | "amp" | "droid" | "openclaw" | "pi")[], since, until, year, sinceTs, untilTs })
      : Promise.resolve({ messages: [], opencodeCount: 0, claudeCount: 0, codexCount: 0, geminiCount: 0, ampCount: 0, droidCount: 0, openclawCount: 0, piCount: 0, processingTimeMs: 0 } as ParsedMessages),
  ]);

  const cursorSync = phase1Results[0].status === "fulfilled" 
    ? phase1Results[0].value 
    : { synced: false, rows: 0, error: "Cursor sync failed" };
  const localMessages = phase1Results[1].status === "fulfilled" 
    ? phase1Results[1].value 
    : null;

  if (includeCursor && cursorSync.error && (cursorSync.synced || hasCursorUsageCache())) {
    // TUI should keep working; just emit a warning.
    const prefix = cursorSync.synced ? "Cursor sync warning" : "Cursor sync failed; using cached data";
    console.warn(`${prefix}: ${cursorSync.error}`);
  }

  const emptyMessages: ParsedMessages = {
    messages: [],
    opencodeCount: 0,
    claudeCount: 0,
    codexCount: 0,
    geminiCount: 0,
    ampCount: 0,
    droidCount: 0,
    openclawCount: 0,
    piCount: 0,
    processingTimeMs: 0,
  };

  setPhase?.("finalizing-report");
  const { report, graph } = await finalizeReportAndGraphAsync({
    localMessages: localMessages || emptyMessages,
    includeCursor: includeCursor && (cursorSync.synced || hasCursorUsageCache()),
    since,
    until,
    year,
    sinceTs,
    untilTs,
  });

  const settings = loadSettings();
  const allModelEntries: ModelEntry[] = report.entries.map(e => ({
    source: e.source,
    model: e.model,
    input: e.input,
    output: e.output,
    cacheWrite: e.cacheWrite,
    cacheRead: e.cacheRead,
    reasoning: e.reasoning,
    total: e.input + e.output + e.cacheWrite + e.cacheRead + e.reasoning,
    cost: e.cost,
  }));
  const modelEntries = settings.includeUnusedModels
    ? allModelEntries
    : allModelEntries.filter(e => e.total > 0);

  const dailyMap = new Map<string, DailyEntry>();
  for (const contrib of graph.contributions) {
    let dateStr = contrib.date;
    
    // When timestamp is available and valid, use it to compute local date
    // This ensures the contribution is bucketed by local timezone, not UTC
    if (contrib.timestamp && contrib.timestamp > 0) {
      const localDate = new Date(contrib.timestamp);
      dateStr = formatDateLocal(localDate);
    }
    
    if (!dailyMap.has(dateStr)) {
      dailyMap.set(dateStr, {
        date: dateStr,
        input: 0,
        output: 0,
        cacheRead: 0,
        cacheWrite: 0,
        total: 0,
        cost: 0,
      });
    }
    const entry = dailyMap.get(dateStr)!;
    entry.input += contrib.tokenBreakdown.input;
    entry.output += contrib.tokenBreakdown.output;
    entry.cacheRead += contrib.tokenBreakdown.cacheRead;
    entry.cacheWrite += contrib.tokenBreakdown.cacheWrite;
    entry.total += contrib.totals.tokens;
    entry.cost += contrib.totals.cost;
  }
  const dailyEntries = Array.from(dailyMap.values()).sort((a, b) => b.date.localeCompare(a.date));

  let maxCost = 1;
  for (const d of dailyEntries) {
    if (d.cost > maxCost) maxCost = d.cost;
  }
  let maxTokens = 1;
  for (const d of dailyEntries) {
    if (d.total > maxTokens) maxTokens = d.total;
  }
  const contributions: ContributionDay[] = dailyEntries.map(d => ({
    date: d.date,
    cost: d.cost,
    tokens: d.total,
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
     let dateStr = contrib.date;
     if (contrib.timestamp && contrib.timestamp > 0) {
       const localDate = new Date(contrib.timestamp);
       dateStr = formatDateLocal(localDate);
     }
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

  const modelTokensMap = new Map<string, { input: number; output: number; cacheRead: number; cacheWrite: number; cost: number }>();
  for (const e of modelEntries) {
    const existing = modelTokensMap.get(e.model) || { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, cost: 0 };
    modelTokensMap.set(e.model, {
      input: existing.input + e.input,
      output: existing.output + e.output,
      cacheRead: existing.cacheRead + e.cacheRead,
      cacheWrite: existing.cacheWrite + e.cacheWrite,
      cost: existing.cost + e.cost,
    });
  }

  const totalCostSum = report.totalCost || 1;
  const topModels: ModelWithPercentage[] = Array.from(modelTokensMap.entries())
    .map(([modelId, data]) => {
      const totalTokens = data.input + data.output + data.cacheRead + data.cacheWrite;
      return {
        modelId,
        percentage: (data.cost / totalCostSum) * 100,
        inputTokens: data.input,
        outputTokens: data.output,
        cacheReadTokens: data.cacheRead,
        cacheWriteTokens: data.cacheWrite,
        totalTokens,
        cost: data.cost,
      };
    })
    .sort((a, b) => b.cost - a.cost);

  const totalReasoning = modelEntries.reduce((sum, e) => sum + e.reasoning, 0);
  const totals: TotalBreakdown = {
    input: report.totalInput,
    output: report.totalOutput,
    cacheWrite: report.totalCacheWrite,
    cacheRead: report.totalCacheRead,
    reasoning: totalReasoning,
    total: report.totalInput + report.totalOutput + report.totalCacheWrite + report.totalCacheRead + totalReasoning,
    cost: report.totalCost,
  };

   const dailyBreakdowns = new Map<string, DailyModelBreakdown>();
   for (const contrib of graph.contributions) {
     let dateStr = contrib.date;
     if (contrib.timestamp && contrib.timestamp > 0) {
       const localDate = new Date(contrib.timestamp);
       dateStr = formatDateLocal(localDate);
     }
    
    const models = contrib.sources.map((source: { modelId: string; source: string; tokens: { input: number; output: number; cacheRead: number; cacheWrite: number; reasoning?: number }; cost: number; messages: number }) => ({
      modelId: source.modelId,
      source: source.source,
      tokens: {
        input: source.tokens.input,
        output: source.tokens.output,
        cacheRead: source.tokens.cacheRead,
        cacheWrite: source.tokens.cacheWrite,
        reasoning: source.tokens.reasoning || 0,
      },
      cost: source.cost,
      messages: source.messages,
    }));
    
    const existing = dailyBreakdowns.get(dateStr);
    if (existing) {
      existing.cost += contrib.totals.cost;
      existing.totalTokens += contrib.totals.tokens;
      existing.models.push(...models);
      existing.models.sort((a, b) => b.cost - a.cost);
    } else {
      dailyBreakdowns.set(dateStr, {
        date: dateStr,
        cost: contrib.totals.cost,
        totalTokens: contrib.totals.tokens,
        models: models.sort((a, b) => b.cost - a.cost),
      });
    }
  }

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
    dailyBreakdowns,
  };
}

export function useData(enabledSources: Accessor<Set<SourceType>>, dateFilters?: DateFilters) {
  const initialSources = enabledSources();
  const initialCachedData = loadCachedData(initialSources);
  const initialCacheIsStale = initialCachedData ? isCacheStale(initialSources) : true;
  
  const [data, setData] = createSignal<TUIData | null>(initialCachedData);
  const [loading, setLoading] = createSignal(!initialCachedData);
  const [error, setError] = createSignal<string | null>(null);
  const [refreshTrigger, setRefreshTrigger] = createSignal(0);
  const [loadingPhase, setLoadingPhase] = createSignal<LoadingPhase>(
    initialCachedData ? (initialCacheIsStale ? "loading-pricing" : "complete") : "idle"
  );
  const [isRefreshing, setIsRefreshing] = createSignal(initialCachedData ? initialCacheIsStale : false);

  const [forceRefresh, setForceRefresh] = createSignal(false);
  let pendingRefresh = false;
  let currentRequestId = 0;

  const refresh = () => {
    if (isRefreshing() || loading()) {
      pendingRefresh = true;
      return;
    }
    setIsRefreshing(true);
    setForceRefresh(true);
    setRefreshTrigger(prev => prev + 1);
  };

  const doLoad = (sources: Set<SourceType>, skipCacheCheck = false) => {
    ++currentRequestId; // Invalidate any in-flight requests immediately
    const shouldSkipCache = skipCacheCheck || forceRefresh();
    
    if (!shouldSkipCache) {
      const cachedData = loadCachedData(sources);
      const cacheIsStale = isCacheStale(sources);
      
      if (cachedData && !cacheIsStale) {
        setData(cachedData);
        setLoading(false);
        setLoadingPhase("complete");
        setIsRefreshing(false);
        return;
      }
      
      if (cachedData) {
        setData(cachedData);
        setLoading(false);
        setIsRefreshing(true);
        setLoadingPhase("idle");
      } else {
        setLoading(true);
        setLoadingPhase("idle");
      }
    } else {
      setIsRefreshing(true);
      setLoadingPhase("idle");
      setForceRefresh(false);
    }
    
    const requestId = currentRequestId;
    setError(null);
    loadData(sources, dateFilters, setLoadingPhase)
      .then((freshData) => {
        if (requestId !== currentRequestId) return;
        setData(freshData);
        saveCachedData(freshData, sources);
      })
      .catch((e: unknown) => {
        if (requestId !== currentRequestId) return;
        setError(e instanceof Error ? e.message : String(e));
      })
      .finally(() => {
        if (requestId !== currentRequestId) return;
        setLoading(false);
        setIsRefreshing(false);
        setLoadingPhase("complete");
        if (pendingRefresh) {
          pendingRefresh = false;
          refresh();
        }
      });
  };

  if (initialCachedData && initialCacheIsStale) {
    doLoad(initialSources, true);
  } else if (!initialCachedData) {
    doLoad(initialSources, false);
  }

  createEffect(on(
    () => [enabledSources(), refreshTrigger()] as const,
    ([sources]) => {
      doLoad(sources);
    },
    { defer: true }
  ));

  return { data, loading, error, refresh, loadingPhase, isRefreshing };
}
