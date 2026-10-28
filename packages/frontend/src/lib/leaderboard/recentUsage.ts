import { unstable_cache } from "next/cache";
import { sql } from "drizzle-orm";
import { db } from "@/lib/db";

/** Days in the leaderboard's "Past 30 days" column, today included. */
export const RECENT_USAGE_DAYS = 30;

/** Most user ids one request may ask for: two pages of 50. */
export const RECENT_USAGE_MAX_USERS = 100;

export interface RecentUsage {
  /** First calendar day of the window (YYYY-MM-DD, UTC). */
  start: string;
  /** Last calendar day of the window (YYYY-MM-DD, UTC), today. */
  end: string;
  /**
   * Daily tokens per user id, oldest day first, exactly RECENT_USAGE_DAYS
   * values. Days with no submitted usage are 0. Users with no usage in the
   * window are present with all zeros, so "no usage" is a fact, not a gap.
   */
  byUser: Record<string, number[]>;
}

const DAY_MS = 86_400_000;

/** The window ending on `now`'s UTC calendar day. */
export function recentUsageWindow(now: Date = new Date()): { start: string; end: string; days: string[] } {
  const endTime = Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), now.getUTCDate());
  const days: string[] = [];
  for (let i = RECENT_USAGE_DAYS - 1; i >= 0; i--) days.push(new Date(endTime - i * DAY_MS).toISOString().slice(0, 10));
  return { start: days[0], end: days[days.length - 1], days };
}

/** Folds (user, day, tokens) rows into one fixed-length series per user. */
export function buildRecentUsage(
  userIds: string[],
  rows: { userId: string; date: string; tokens: number | string | null }[],
  now: Date = new Date(),
): RecentUsage {
  const { start, end, days } = recentUsageWindow(now);
  const index = new Map(days.map((day, i) => [day, i]));
  const byUser: Record<string, number[]> = {};
  for (const id of userIds) byUser[id] = new Array(RECENT_USAGE_DAYS).fill(0);
  for (const row of rows) {
    const series = byUser[row.userId];
    const i = index.get(String(row.date).slice(0, 10));
    if (!series || i === undefined) continue;
    series[i] += Number(row.tokens) || 0;
  }
  return { start, end, byUser };
}

async function fetchRecentUsage(userIds: string[], end: string): Promise<RecentUsage> {
  const now = new Date(`${end}T00:00:00Z`);
  const { start } = recentUsageWindow(now);
  if (userIds.length === 0) return buildRecentUsage([], [], now);
  const rows = await db.execute<{ userId: string; date: string; tokens: string }>(sql`
    SELECT s.user_id AS "userId", d.date::text AS "date", SUM(d.tokens) AS "tokens"
    FROM daily_breakdown d
    INNER JOIN submissions s ON d.submission_id = s.id
    WHERE s.user_id IN (${sql.join(userIds.map((id) => sql`${id}`), sql`, `)})
      AND d.date >= ${start} AND d.date <= ${end}
    GROUP BY s.user_id, d.date
  `);
  return buildRecentUsage(userIds, Array.from(rows), now);
}

/**
 * Daily token totals over the last RECENT_USAGE_DAYS UTC days for the given
 * users. Cached per (user set, day) for five minutes under the leaderboard
 * tag, so moderation and data deletion that revalidate "leaderboard" also
 * clear these series.
 */
export function getRecentUsage(userIds: string[], now: Date = new Date()): Promise<RecentUsage> {
  const ids = Array.from(new Set(userIds)).sort().slice(0, RECENT_USAGE_MAX_USERS);
  const { end } = recentUsageWindow(now);
  return unstable_cache(
    () => fetchRecentUsage(ids, end),
    ["leaderboard-recent-usage", end, ids.join(",")],
    { tags: ["leaderboard"], revalidate: 300 },
  )();
}
