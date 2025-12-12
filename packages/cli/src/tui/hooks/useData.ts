import { useState, useEffect, useCallback } from "react";
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

export interface TUIData {
  modelEntries: ModelEntry[];
  dailyEntries: DailyEntry[];
  contributions: ContributionDay[];
  stats: Stats;
  totalCost: number;
  modelCount: number;
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
      const prev = new Date(sortedDates[i - 1]);
      const curr = new Date(sortedDates[i]);
      const diffDays = (curr.getTime() - prev.getTime()) / (1000 * 60 * 60 * 24);
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

  return {
    modelEntries,
    dailyEntries,
    contributions,
    stats,
    totalCost: report.totalCost,
    modelCount: modelEntries.length,
  };
}

export function useData(enabledSources: Set<SourceType>) {
  const [data, setData] = useState<TUIData | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(() => {
    setLoading(true);
    setError(null);
    loadData(enabledSources)
      .then(setData)
      .catch((e) => setError(e.message))
      .finally(() => setLoading(false));
  }, [enabledSources]);

  useEffect(() => {
    refresh();
  }, [refresh]);

  return { data, loading, error, refresh };
}
