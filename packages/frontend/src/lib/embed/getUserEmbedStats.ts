import { unstable_cache } from "next/cache";
import { db, users, submissions, dailyBreakdown } from "@/lib/db";
import {
  USERNAME_LOOKUP_LIMIT,
  getSingleUsernameMatch,
  normalizeUsernameCacheKey,
  usernameEqualsIgnoreCase,
} from "@/lib/db/usernameLookup";
import { eq, sql, and, gte, lte } from "drizzle-orm";
import {
  getContributionIntensity,
  getContributionWindow,
  getEmbedPeriodDateRange,
  type EmbedPeriod,
  type EmbedPeriodDateRange,
} from "./embedShared";

export type EmbedSortBy = "tokens" | "cost";

export interface EmbedContributionDay {
  date: string;
  totalTokens: number;
  totalCost: number;
  intensity: 0 | 1 | 2 | 3 | 4;
}

export interface UserEmbedStats {
  period: EmbedPeriod;
  dateRange: EmbedPeriodDateRange | null;
  user: {
    id: string;
    username: string;
    displayName: string | null;
    avatarUrl: string | null;
  };
  stats: {
    totalTokens: number;
    totalCost: number;
    submissionCount: number;
    rank: number | null;
    /** Total number of ranked users, for rendering "rank N of total". */
    rankTotal?: number | null;
    updatedAt: string | null;
    /** True once any accepted submission carried a backfill provenance tag. */
    hasBackfill: boolean;
  };
}

async function fetchUserEmbedStats(
  username: string,
  sortBy: EmbedSortBy,
  period: EmbedPeriod,
): Promise<UserEmbedStats | null> {
  const matchingUsers = await db
    .select({
      id: users.id,
      username: users.username,
      displayName: users.displayName,
      avatarUrl: users.avatarUrl,
      totalTokens: sql<number>`COALESCE(${submissions.totalTokens}, 0)`,
      totalCost: sql<number>`COALESCE(CAST(${submissions.totalCost} AS DECIMAL(18,4)), 0)`,
      submissionCount: sql<number>`COALESCE(${submissions.submitCount}, 0)`,
      latestDate: submissions.dateEnd,
      updatedAt: submissions.updatedAt,
      hasBackfill: sql<boolean>`COALESCE(${submissions.hasBackfill}, false)`,
    })
    .from(users)
    .leftJoin(submissions, eq(submissions.userId, users.id))
    .where(usernameEqualsIgnoreCase(username))
    .limit(USERNAME_LOOKUP_LIMIT);
  const result = getSingleUsernameMatch(matchingUsers, username);

  if (!result) {
    return null;
  }

  let rank: number | null = null;
  let rankTotal: number | null = null;
  const dateRange = getEmbedPeriodDateRange(period, result.latestDate);

  const rankingValue =
    sortBy === "cost"
      ? Number(result.totalCost) || 0
      : Number(result.totalTokens) || 0;

  if (rankingValue > 0) {
    // Both the rank and the "of N" denominator count rankable users only, so a
    // hidden account neither holds a position nor inflates the total. A hidden
    // user is absent from the CTE, so this returns no row and rank stays null.
    const rankResult = await db.execute<{ rank: number; total: number }>(sql`
      WITH rankable AS (
        SELECT s.*
        FROM submissions s
        JOIN users u ON u.id = s.user_id
        WHERE u.leaderboard_hidden = false
      ),
      ranked AS (
        SELECT
          user_id,
          RANK() OVER (
            ORDER BY
              ${
                sortBy === "cost"
                  ? sql`CAST(total_cost AS DECIMAL(18,4)) DESC`
                  : sql`total_tokens DESC`
              }
          ) AS rank
        FROM rankable
      )
      SELECT rank, (SELECT COUNT(*)::int FROM rankable) AS total
      FROM ranked WHERE user_id = ${result.id}
    `);

    const rankRow = (
      rankResult as unknown as { rank: number; total: number }[]
    )[0];
    // Coerced because RANK() is a Postgres bigint, which postgres-js hands
    // back as a string — so without this the returned value is "1", not 1, and
    // the `number | null` on UserEmbedStats is wrong. publicProfileData.ts
    // compensates for the same thing at its call site.
    //
    // Number(undefined) is NaN and falls through to null, so a missing row
    // behaves as before.
    rank = Number(rankRow?.rank) || null;
    rankTotal = Number(rankRow?.total) || null;
  }

  let totalTokens = Number(result.totalTokens) || 0;
  let totalCost = Number(result.totalCost) || 0;

  if (dateRange) {
    const periodResult = await db.execute<{
      totalTokens: number;
      totalCost: number;
    }>(sql`
      SELECT
        COALESCE(SUM(d.tokens), 0) AS "totalTokens",
        COALESCE(SUM(CAST(d.cost AS DECIMAL(18,4))), 0) AS "totalCost"
      FROM daily_breakdown d
      INNER JOIN submissions s ON d.submission_id = s.id
      WHERE s.user_id = ${result.id}
        AND d.date >= ${dateRange.start}
        AND d.date <= ${dateRange.end}
    `);
    const periodRow = periodResult[0];
    totalTokens = Number(periodRow?.totalTokens) || 0;
    totalCost = Number(periodRow?.totalCost) || 0;
  }

  return {
    period,
    dateRange,
    user: {
      id: result.id,
      username: result.username,
      displayName: result.displayName,
      avatarUrl: result.avatarUrl,
    },
    stats: {
      totalTokens,
      totalCost,
      submissionCount: Number(result.submissionCount) || 0,
      rank,
      rankTotal,
      updatedAt: result.updatedAt?.toISOString() || null,
      hasBackfill: Boolean(result.hasBackfill),
    },
  };
}

export function getUserEmbedStats(
  username: string,
  sortBy: EmbedSortBy = "tokens",
  period: EmbedPeriod = "all",
): Promise<UserEmbedStats | null> {
  const usernameCacheKey = normalizeUsernameCacheKey(username);

  return unstable_cache(
    () => fetchUserEmbedStats(username, sortBy, period),
    [`embed-user:${usernameCacheKey}:${sortBy}:${period}`],
    {
      tags: [
        `user:${usernameCacheKey}`,
        `embed-user:${usernameCacheKey}`,
        `embed-user:${usernameCacheKey}:${sortBy}`,
        `embed-user:${usernameCacheKey}:${sortBy}:${period}`,
      ],
      revalidate: 60,
    },
  )();
}

async function fetchUserEmbedContributions(
  username: string,
  dateRange: EmbedPeriodDateRange | null,
): Promise<EmbedContributionDay[] | null> {
  const matchingUsers = await db
    .select({ id: users.id })
    .from(users)
    .where(usernameEqualsIgnoreCase(username))
    .limit(USERNAME_LOOKUP_LIMIT);
  const user = getSingleUsernameMatch(matchingUsers, username);

  if (!user) return null;

  let cutoff: string;
  if (dateRange) {
    cutoff = dateRange.start;
  } else {
    // Use UTC-based date and include a 7-day buffer before "one year ago"
    // so that all dates visible in the first week of the contribution grid are included.
    const today = new Date();
    const cutoffDate = new Date(
      Date.UTC(
        today.getUTCFullYear() - 1,
        today.getUTCMonth(),
        today.getUTCDate(),
      ),
    );
    cutoffDate.setUTCDate(cutoffDate.getUTCDate() - 7);
    cutoff = cutoffDate.toISOString().split("T")[0];
  }

  const contributionFilter = dateRange
    ? and(
        eq(submissions.userId, user.id),
        gte(dailyBreakdown.date, dateRange.start),
        lte(dailyBreakdown.date, dateRange.end),
      )
    : and(eq(submissions.userId, user.id), gte(dailyBreakdown.date, cutoff));

  const rows = await db
    .select({
      date: dailyBreakdown.date,
      tokens: sql<number>`sum(${dailyBreakdown.tokens})`.as("tokens"),
      cost: sql<number>`sum(${dailyBreakdown.cost})`.as("cost"),
    })
    .from(dailyBreakdown)
    .innerJoin(submissions, eq(dailyBreakdown.submissionId, submissions.id))
    .where(contributionFilter)
    .groupBy(dailyBreakdown.date)
    .orderBy(dailyBreakdown.date);

  if (rows.length === 0) return [];

  const contributions: EmbedContributionDay[] = rows.map((row) => ({
    date: row.date,
    totalTokens: Number(row.tokens) || 0,
    totalCost: Number(row.cost) || 0,
    intensity: 0,
  }));
  const referenceDate = dateRange
    ? new Date(`${dateRange.end}T00:00:00.000Z`)
    : new Date();
  const contributionWindow = getContributionWindow(
    contributions,
    referenceDate,
  );
  const scopedDates = new Set(contributionWindow.days.map(({ date }) => date));
  const maxTokens = Math.max(
    0,
    ...contributionWindow.days.map(({ totalTokens }) =>
      Math.max(0, totalTokens),
    ),
  );

  return contributions.map((contribution) => ({
    ...contribution,
    intensity: scopedDates.has(contribution.date)
      ? getContributionIntensity(contribution.totalTokens, maxTokens)
      : 0,
  }));
}

export function getUserEmbedContributions(
  username: string,
  dateRange: EmbedPeriodDateRange | null = null,
): Promise<EmbedContributionDay[] | null> {
  const usernameCacheKey = normalizeUsernameCacheKey(username);
  const rangeKey = dateRange
    ? `${dateRange.start}:${dateRange.end}`
    : "all";

  return unstable_cache(
    () => fetchUserEmbedContributions(username, dateRange),
    [`embed-contrib:${usernameCacheKey}:${rangeKey}`],
    {
      tags: [
        `user:${usernameCacheKey}`,
        `embed-contrib:${usernameCacheKey}`,
        `embed-contrib:${usernameCacheKey}:${rangeKey}`,
      ],
      revalidate: 60,
    },
  )();
}
