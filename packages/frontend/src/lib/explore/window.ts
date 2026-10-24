/**
 * Period windows for the explore rollups. Pure functions over ISO dates (UTC).
 *
 * - "7d" / "30d" end at the latest UTC date that has data, not at "today", so a
 *   quiet morning does not shrink the window to a partial day of nothing.
 * - The previous window has the same length and ends the day before `start`.
 * - "all" starts at the first date with data and has no previous window.
 */
import type { ExplorePeriod } from "./types";

export interface ExploreWindow {
  period: ExplorePeriod;
  /** Inclusive. */
  start: string;
  /** Inclusive. */
  end: string;
  /** Same-length previous window, or null for "all". */
  previous: { start: string; end: string } | null;
  /** Number of days in [start, end]. */
  days: number;
}

const DAY_MS = 86_400_000;

export function periodDays(period: ExplorePeriod): number | null {
  return period === "7d" ? 7 : period === "30d" ? 30 : null;
}

function parseIsoDate(iso: string): number {
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(iso);
  if (!match) throw new Error(`Invalid ISO date: ${iso}`);
  return Date.UTC(Number(match[1]), Number(match[2]) - 1, Number(match[3]));
}

export function toIsoDate(ms: number): string {
  return new Date(ms).toISOString().slice(0, 10);
}

export function addDays(iso: string, days: number): string {
  return toIsoDate(parseIsoDate(iso) + days * DAY_MS);
}

/** Inclusive day count between two ISO dates (1 when equal). */
export function daysBetweenInclusive(start: string, end: string): number {
  return Math.round((parseIsoDate(end) - parseIsoDate(start)) / DAY_MS) + 1;
}

/** Every ISO date in [start, end], inclusive. Empty when end < start. */
export function enumerateDates(start: string, end: string): string[] {
  const out: string[] = [];
  const last = parseIsoDate(end);
  for (let t = parseIsoDate(start); t <= last; t += DAY_MS) out.push(toIsoDate(t));
  return out;
}

/**
 * Latest date allowed to anchor a window. A user's local calendar can run one
 * day ahead of UTC, so the anchor may be tomorrow (UTC) but never later: a
 * far-future row from a broken clock must not drag every window forward.
 */
export function maxAnchorDate(now: Date): string {
  const today = toIsoDate(Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), now.getUTCDate()));
  return addDays(today, 1);
}

/**
 * Resolve the window for `period` given the data bounds. With no data at all
 * the window collapses to `fallbackEnd` so callers still get valid dates.
 */
export function resolveWindow(
  period: ExplorePeriod,
  bounds: { firstDate: string | null; latestDate: string | null },
  fallbackEnd: string,
): ExploreWindow {
  const end = bounds.latestDate ?? fallbackEnd;
  const length = periodDays(period);
  if (length === null) {
    const start = bounds.firstDate && bounds.firstDate <= end ? bounds.firstDate : end;
    return { period, start, end, previous: null, days: daysBetweenInclusive(start, end) };
  }
  const start = addDays(end, -(length - 1));
  const previousEnd = addDays(start, -1);
  const previousStart = addDays(previousEnd, -(length - 1));
  return { period, start, end, previous: { start: previousStart, end: previousEnd }, days: length };
}

/** First date the rollup query has to read: previous.start, or start for "all". */
export function queryStart(window: ExploreWindow): string {
  return window.previous?.start ?? window.start;
}

/** activeUsers / accounts, 0 when there are no accounts. */
export function adoption(activeUsers: number, accounts: number): number {
  return accounts > 0 ? activeUsers / accounts : 0;
}

/** Share of tokens whose model was identified; 0 when there were no tokens. */
export function identifiedShare(identifiedTokens: number, totalTokens: number): number {
  if (totalTokens <= 0) return 0;
  return Math.min(1, Math.max(0, identifiedTokens / totalTokens));
}

/** A model/agent is new in a window when it has no usage before the window start. */
export function isNewInWindow(window: ExploreWindow, firstSeen: string | null): boolean {
  if (window.previous === null || firstSeen === null) return false;
  return firstSeen >= window.start;
}
