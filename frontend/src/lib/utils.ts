/**
 * Utility functions for contribution graph
 * Includes date grouping, filtering, formatting, and data manipulation
 */

import {
  format,
  parseISO,
  startOfWeek,
  addDays,
  startOfYear,
  endOfYear,
  eachDayOfInterval,
  isSameDay,
} from "date-fns";
import type {
  DailyContribution,
  TokenContributionData,
  SourceType,
  WeekData,
  SourceContribution,
  TokenBreakdown,
} from "./types";

// =====================================
// Date Grouping Functions
// =====================================

/**
 * Group contributions by week for grid rendering
 * Returns 53 weeks x 7 days structure (Sunday-first)
 * Empty days are filled with null
 */
export function groupByWeek(
  contributions: DailyContribution[],
  year: string
): WeekData[] {
  const weeks: WeekData[] = [];
  const contributionMap = new Map<string, DailyContribution>();

  // Create a map for quick lookup
  for (const c of contributions) {
    contributionMap.set(c.date, c);
  }

  // Get the first Sunday of the year (or last Sunday of previous year)
  const yearStart = startOfYear(parseISO(`${year}-01-01`));
  const firstSunday = startOfWeek(yearStart, { weekStartsOn: 0 });

  // Generate 53 weeks
  for (let weekIndex = 0; weekIndex < 53; weekIndex++) {
    const days: (DailyContribution | null)[] = [];

    for (let dayIndex = 0; dayIndex < 7; dayIndex++) {
      const currentDate = addDays(firstSunday, weekIndex * 7 + dayIndex);
      const dateStr = format(currentDate, "yyyy-MM-dd");
      const contribution = contributionMap.get(dateStr) || null;
      days.push(contribution);
    }

    weeks.push({ weekIndex, days });
  }

  return weeks;
}

/**
 * Get all dates in a year range for filling empty days
 */
export function getYearDates(year: string): Date[] {
  const start = startOfYear(parseISO(`${year}-01-01`));
  const end = endOfYear(parseISO(`${year}-12-31`));
  return eachDayOfInterval({ start, end });
}

/**
 * Fill missing days with empty contributions
 */
export function fillMissingDays(
  contributions: DailyContribution[],
  year: string
): DailyContribution[] {
  const existingDates = new Set(contributions.map((c) => c.date));
  const yearDates = getYearDates(year);
  const result: DailyContribution[] = [...contributions];

  for (const date of yearDates) {
    const dateStr = format(date, "yyyy-MM-dd");
    if (!existingDates.has(dateStr)) {
      result.push(createEmptyContribution(dateStr));
    }
  }

  return result.sort((a, b) => a.date.localeCompare(b.date));
}

/**
 * Create an empty contribution for a date
 */
function createEmptyContribution(date: string): DailyContribution {
  return {
    date,
    totals: { tokens: 0, cost: 0, messages: 0 },
    intensity: 0,
    tokenBreakdown: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, reasoning: 0 },
    sources: [],
  };
}

// =====================================
// Filtering Functions
// =====================================

/**
 * Filter data by source types
 * Returns a new TokenContributionData with filtered sources
 */
export function filterBySource(
  data: TokenContributionData,
  sources: SourceType[]
): TokenContributionData {
  if (sources.length === 0) return data;

  const sourceSet = new Set(sources);

  const filteredContributions = data.contributions.map((day) => {
    const filteredSources = day.sources.filter((s) => sourceSet.has(s.source));
    return recalculateDayTotals({ ...day, sources: filteredSources });
  });

  return {
    ...data,
    contributions: recalculateIntensity(filteredContributions),
    summary: recalculateSummary(filteredContributions, sources),
  };
}

/**
 * Filter data by model IDs
 * Returns a new TokenContributionData with filtered models
 */
export function filterByModel(
  data: TokenContributionData,
  models: string[]
): TokenContributionData {
  if (models.length === 0) return data;

  const modelSet = new Set(models);

  const filteredContributions = data.contributions.map((day) => {
    const filteredSources = day.sources.filter((s) => modelSet.has(s.modelId));
    return recalculateDayTotals({ ...day, sources: filteredSources });
  });

  // Get unique sources from filtered data
  const filteredSourceSet = new Set<SourceType>();
  for (const c of filteredContributions) {
    for (const s of c.sources) {
      filteredSourceSet.add(s.source);
    }
  }

  return {
    ...data,
    contributions: recalculateIntensity(filteredContributions),
    summary: recalculateSummary(filteredContributions, Array.from(filteredSourceSet)),
  };
}

/**
 * Filter contributions by year
 */
export function filterByYear(
  contributions: DailyContribution[],
  year: string
): DailyContribution[] {
  return contributions.filter((c) => c.date.startsWith(year));
}

// =====================================
// Recalculation Functions
// =====================================

/**
 * Recalculate day totals from sources
 */
function recalculateDayTotals(day: DailyContribution): DailyContribution {
  const tokenBreakdown: TokenBreakdown = {
    input: 0,
    output: 0,
    cacheRead: 0,
    cacheWrite: 0,
    reasoning: 0,
  };

  let totalCost = 0;
  let totalMessages = 0;

  for (const source of day.sources) {
    tokenBreakdown.input += source.tokens.input;
    tokenBreakdown.output += source.tokens.output;
    tokenBreakdown.cacheRead += source.tokens.cacheRead;
    tokenBreakdown.cacheWrite += source.tokens.cacheWrite;
    tokenBreakdown.reasoning += source.tokens.reasoning;
    totalCost += source.cost;
    totalMessages += source.messages;
  }

  const totalTokens =
    tokenBreakdown.input +
    tokenBreakdown.output +
    tokenBreakdown.cacheRead +
    tokenBreakdown.cacheWrite +
    tokenBreakdown.reasoning;

  return {
    ...day,
    totals: { tokens: totalTokens, cost: totalCost, messages: totalMessages },
    tokenBreakdown,
    // Keep original intensity, will be recalculated later
    intensity: day.intensity,
  };
}

/**
 * Recalculate intensity for all contributions based on max cost
 */
export function recalculateIntensity(
  contributions: DailyContribution[]
): DailyContribution[] {
  const maxCost = Math.max(...contributions.map((c) => c.totals.cost), 0);

  return contributions.map((c) => ({
    ...c,
    intensity: calculateIntensity(c.totals.cost, maxCost),
  }));
}

/**
 * Calculate intensity grade based on cost ratio
 * Uses quartile-based thresholds
 */
function calculateIntensity(cost: number, maxCost: number): 0 | 1 | 2 | 3 | 4 {
  if (cost === 0 || maxCost === 0) return 0;
  const ratio = cost / maxCost;
  if (ratio >= 0.75) return 4;
  if (ratio >= 0.5) return 3;
  if (ratio >= 0.25) return 2;
  return 1;
}

/**
 * Recalculate summary from contributions
 */
function recalculateSummary(
  contributions: DailyContribution[],
  sources: SourceType[]
): TokenContributionData["summary"] {
  const activeDays = contributions.filter((c) => c.totals.cost > 0);
  const totalCost = activeDays.reduce((sum, c) => sum + c.totals.cost, 0);
  const totalTokens = activeDays.reduce((sum, c) => sum + c.totals.tokens, 0);
  const maxCost = Math.max(...contributions.map((c) => c.totals.cost), 0);

  // Collect unique models
  const modelSet = new Set<string>();
  for (const c of contributions) {
    for (const s of c.sources) {
      modelSet.add(s.modelId);
    }
  }

  return {
    totalTokens,
    totalCost,
    totalDays: contributions.length,
    activeDays: activeDays.length,
    averagePerDay: activeDays.length > 0 ? totalCost / activeDays.length : 0,
    maxCostInSingleDay: maxCost,
    sources,
    models: Array.from(modelSet),
  };
}

// =====================================
// Formatting Functions
// =====================================

/**
 * Format token count with K/M/B suffixes
 */
export function formatTokenCount(count: number): string {
  if (count >= 1_000_000_000) {
    return `${(count / 1_000_000_000).toFixed(1)}B`;
  }
  if (count >= 1_000_000) {
    return `${(count / 1_000_000).toFixed(1)}M`;
  }
  if (count >= 1_000) {
    return `${(count / 1_000).toFixed(1)}K`;
  }
  return count.toLocaleString();
}

/**
 * Format currency with dollar sign
 */
export function formatCurrency(amount: number): string {
  return `$${amount.toFixed(2)}`;
}

/**
 * Format date for display (e.g., "Dec 1, 2024")
 */
export function formatDate(dateStr: string): string {
  return format(parseISO(dateStr), "MMM d, yyyy");
}

/**
 * Format date for full display (e.g., "December 1, 2024")
 */
export function formatDateFull(dateStr: string): string {
  return format(parseISO(dateStr), "MMMM d, yyyy");
}

/**
 * Get day of week name (e.g., "Monday")
 */
export function getDayName(dateStr: string): string {
  return format(parseISO(dateStr), "EEEE");
}

// =====================================
// Statistics Functions
// =====================================

/**
 * Calculate current streak (consecutive days with activity)
 */
export function calculateCurrentStreak(contributions: DailyContribution[]): number {
  const sorted = [...contributions]
    .filter((c) => c.totals.cost > 0)
    .sort((a, b) => b.date.localeCompare(a.date)); // Most recent first

  if (sorted.length === 0) return 0;

  let streak = 0;
  const today = new Date();
  let expectedDate = today;

  for (const c of sorted) {
    const contributionDate = parseISO(c.date);
    
    // Check if this is the expected date or the day before
    if (isSameDay(contributionDate, expectedDate) || 
        isSameDay(contributionDate, addDays(expectedDate, -1))) {
      streak++;
      expectedDate = addDays(contributionDate, -1);
    } else {
      break;
    }
  }

  return streak;
}

/**
 * Calculate longest streak
 */
export function calculateLongestStreak(contributions: DailyContribution[]): number {
  const activeDates = contributions
    .filter((c) => c.totals.cost > 0)
    .map((c) => c.date)
    .sort();

  if (activeDates.length === 0) return 0;

  let longestStreak = 1;
  let currentStreak = 1;

  for (let i = 1; i < activeDates.length; i++) {
    const prevDate = parseISO(activeDates[i - 1]);
    const currDate = parseISO(activeDates[i]);
    const dayDiff = Math.round(
      (currDate.getTime() - prevDate.getTime()) / (1000 * 60 * 60 * 24)
    );

    if (dayDiff === 1) {
      currentStreak++;
      longestStreak = Math.max(longestStreak, currentStreak);
    } else {
      currentStreak = 1;
    }
  }

  return longestStreak;
}

/**
 * Find the day with highest cost
 */
export function findBestDay(
  contributions: DailyContribution[]
): DailyContribution | null {
  if (contributions.length === 0) return null;

  return contributions.reduce((best, current) =>
    current.totals.cost > best.totals.cost ? current : best
  );
}

// =====================================
// Color Utilities
// =====================================

/**
 * Parse hex color to RGB object
 */
export function hexToRgb(hex: string): { r: number; g: number; b: number } | null {
  const result = /^#?([a-f\d]{2})([a-f\d]{2})([a-f\d]{2})$/i.exec(hex);
  return result
    ? {
        r: parseInt(result[1], 16),
        g: parseInt(result[2], 16),
        b: parseInt(result[3], 16),
      }
    : null;
}

/**
 * Convert hex color to number for obelisk.js
 */
export function hexToNumber(hex: string): number {
  return parseInt(hex.replace("#", ""), 16);
}

// =====================================
// Data Validation
// =====================================

/**
 * Validate TokenContributionData structure
 */
export function isValidContributionData(data: unknown): data is TokenContributionData {
  if (!data || typeof data !== "object") return false;

  const d = data as Record<string, unknown>;

  return (
    typeof d.meta === "object" &&
    typeof d.summary === "object" &&
    Array.isArray(d.years) &&
    Array.isArray(d.contributions)
  );
}

// =====================================
// Source Grouping
// =====================================

/**
 * Group sources by source type for breakdown display
 */
export function groupSourcesByType(
  sources: SourceContribution[]
): Map<SourceType, SourceContribution[]> {
  const grouped = new Map<SourceType, SourceContribution[]>();

  for (const source of sources) {
    const existing = grouped.get(source.source) || [];
    existing.push(source);
    grouped.set(source.source, existing);
  }

  return grouped;
}

/**
 * Sort sources by cost descending within each group
 */
export function sortSourcesByCost(
  sources: SourceContribution[]
): SourceContribution[] {
  return [...sources].sort((a, b) => b.cost - a.cost);
}
