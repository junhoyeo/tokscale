import { unstable_cache } from "next/cache";
import { db, users, submissions } from "@/lib/db";
import {
  USERNAME_LOOKUP_LIMIT,
  getSingleUsernameMatch,
  normalizeUsernameCacheKey,
  usernameEqualsIgnoreCase,
} from "@/lib/db/usernameLookup";
import { eq, sql } from "drizzle-orm";
import type { LeaderboardData, LeaderboardUser, Period, SortBy } from "@/lib/leaderboard/types";

export type { LeaderboardData, LeaderboardUser, Period, SortBy } from "@/lib/leaderboard/types";

interface PeriodDateRange {
  start: string;
  end: string;
}

type LeaderboardQueryResult = Record<string, unknown> & {
  users: unknown;
  totalUsers: number | string | null;
  totalTokens: number | string | null;
  totalCost: number | string | null;
  uniqueUsers: number | string | null;
};

type RankedLeaderboardDbRow = Record<string, unknown> & {
  rank: number | string | null;
  userId: string;
  username: string;
  displayName: string | null;
  avatarUrl: string | null;
  totalTokens: number | string | null;
  totalCost: number | string | null;
};

function toUtcDateString(date: Date): string {
  return date.toISOString().slice(0, 10);
}

function getPeriodDateRange(
  period: Period,
  now: Date = new Date(),
  customFrom?: string,
  customTo?: string
): PeriodDateRange | null {
  if (period === "all") {
    return null;
  }

  if (period === "custom") {
    if (!customFrom || !customTo) {
      return null;
    }
    return { start: customFrom, end: customTo };
  }

  const end = new Date(
    Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), now.getUTCDate())
  );

  if (period === "week") {
    const start = new Date(end);
    start.setUTCDate(start.getUTCDate() - 6);
    return {
      start: toUtcDateString(start),
      end: toUtcDateString(end),
    };
  }

  if (period === "last-month") {
    const lastMonthEnd = new Date(Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), 0));
    const lastMonthStart = new Date(Date.UTC(lastMonthEnd.getUTCFullYear(), lastMonthEnd.getUTCMonth(), 1));
    return {
      start: toUtcDateString(lastMonthStart),
      end: toUtcDateString(lastMonthEnd),
    };
  }

  const start = new Date(Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), 1));
  return {
    start: toUtcDateString(start),
    end: toUtcDateString(end),
  };
}

function getSearchPattern(search: string): string {
  const escapedSearch = search.toLowerCase().replace(/[%_\\]/g, "\\$&");
  return `%${escapedSearch}%`;
}

function mapLeaderboardUser(row: RankedLeaderboardDbRow): LeaderboardUser {
  return {
    rank: Number(row.rank) || 0,
    userId: row.userId,
    username: row.username,
    displayName: row.displayName,
    avatarUrl: row.avatarUrl,
    totalTokens: Number(row.totalTokens) || 0,
    totalCost: Number(row.totalCost) || 0,
  };
}

function parseLeaderboardUsers(value: unknown): LeaderboardUser[] {
  let rows = value;

  if (typeof rows === "string") {
    try {
      rows = JSON.parse(rows);
    } catch {
      return [];
    }
  }

  if (!Array.isArray(rows)) {
    return [];
  }

  return rows.map((row) => mapLeaderboardUser(row as RankedLeaderboardDbRow));
}

function buildLeaderboardData(
  row: LeaderboardQueryResult | undefined,
  page: number,
  limit: number,
  period: Period,
  sortBy: SortBy
): LeaderboardData {
  const totalUsers = Number(row?.totalUsers) || 0;
  const totalPages = Math.ceil(totalUsers / limit);

  return {
    users: parseLeaderboardUsers(row?.users),
    pagination: {
      page,
      limit,
      totalUsers,
      totalPages,
      hasNext: page < totalPages,
      hasPrev: page > 1,
    },
    stats: {
      totalTokens: Number(row?.totalTokens) || 0,
      totalCost: Number(row?.totalCost) || 0,
      uniqueUsers: Number(row?.uniqueUsers) || 0,
    },
    period,
    sortBy,
  };
}

/**
 * Period rankings intentionally use ROW_NUMBER over the full display ordering.
 * This preserves the sequential ranks that the former in-memory sort assigned
 * when primary-metric ties were resolved by the secondary metric and username.
 */
async function fetchPeriodLeaderboardData(
  period: Exclude<Period, "all">,
  page: number,
  limit: number,
  sortBy: SortBy,
  search: string,
  customFrom?: string,
  customTo?: string
): Promise<LeaderboardData> {
  const dateRange = getPeriodDateRange(period, new Date(), customFrom, customTo);

  if (!dateRange) {
    return buildLeaderboardData(undefined, page, limit, period, sortBy);
  }

  const offset = (page - 1) * limit;
  const primaryOrderColumn = sortBy === "cost" ? sql`total_cost` : sql`total_tokens`;
  const secondaryOrderColumn = sortBy === "cost" ? sql`total_tokens` : sql`total_cost`;
  const searchCondition = search
    ? sql`LOWER(username) LIKE ${getSearchPattern(search)} ESCAPE '\\' OR LOWER(COALESCE(display_name, '')) LIKE ${getSearchPattern(search)} ESCAPE '\\'`
    : sql`TRUE`;

  const result = await db.execute<LeaderboardQueryResult>(sql`
    WITH aggregated AS (
      SELECT
        submissions.user_id AS user_id,
        users.username AS username,
        users.display_name AS display_name,
        users.avatar_url AS avatar_url,
        SUM(daily_breakdown.tokens) AS total_tokens,
        SUM(CAST(daily_breakdown.cost AS DECIMAL(18,4))) AS total_cost
      FROM daily_breakdown
      INNER JOIN submissions ON daily_breakdown.submission_id = submissions.id
      INNER JOIN users ON submissions.user_id = users.id
      WHERE daily_breakdown.date >= ${dateRange.start}
        AND daily_breakdown.date <= ${dateRange.end}
      GROUP BY submissions.user_id, users.username, users.display_name, users.avatar_url
    ),
    ranked AS (
      SELECT
        aggregated.*,
        ROW_NUMBER() OVER (
          ORDER BY ${primaryOrderColumn} DESC, ${secondaryOrderColumn} DESC, LOWER(username) ASC, user_id ASC
        ) AS rank,
        SUM(total_tokens) OVER () AS total_tokens_all,
        SUM(total_cost) OVER () AS total_cost_all,
        COUNT(*) OVER () AS unique_users_all
      FROM aggregated
    ),
    filtered AS (
      SELECT ranked.*, COUNT(*) OVER () AS filtered_users
      FROM ranked
      WHERE ${searchCondition}
    ),
    paged AS (
      SELECT *
      FROM filtered
      ORDER BY rank ASC
      LIMIT ${limit}
      OFFSET ${offset}
    )
    SELECT
      COALESCE(
        (
          SELECT json_agg(
            json_build_object(
              'rank', rank,
              'userId', user_id,
              'username', username,
              'displayName', display_name,
              'avatarUrl', avatar_url,
              'totalTokens', total_tokens,
              'totalCost', total_cost
            )
            ORDER BY rank ASC
          )
          FROM paged
        ),
        '[]'::json
      ) AS users,
      COALESCE((SELECT filtered_users FROM filtered LIMIT 1), 0)::int AS "totalUsers",
      COALESCE((SELECT total_tokens_all FROM ranked LIMIT 1), 0) AS "totalTokens",
      COALESCE((SELECT total_cost_all FROM ranked LIMIT 1), 0) AS "totalCost",
      COALESCE((SELECT unique_users_all FROM ranked LIMIT 1), 0)::int AS "uniqueUsers"
  `);

  return buildLeaderboardData(result[0], page, limit, period, sortBy);
}

async function fetchAllTimeLeaderboardData(
  page: number,
  limit: number,
  sortBy: SortBy,
  search: string
): Promise<LeaderboardData> {
  const offset = (page - 1) * limit;
  const primaryOrderColumn = sortBy === "cost" ? sql`total_cost` : sql`total_tokens`;
  const secondaryOrderColumn = sortBy === "cost" ? sql`total_tokens` : sql`total_cost`;
  const searchCondition = search
    ? sql`LOWER(username) LIKE ${getSearchPattern(search)} ESCAPE '\\' OR LOWER(COALESCE(display_name, '')) LIKE ${getSearchPattern(search)} ESCAPE '\\'`
    : sql`TRUE`;

  // submissions.user_id is unique. Keep this path row-based rather than
  // grouping every submission on each request.
  const result = await db.execute<LeaderboardQueryResult>(sql`
    WITH ranked AS (
      SELECT
        submissions.user_id AS user_id,
        users.username AS username,
        users.display_name AS display_name,
        users.avatar_url AS avatar_url,
        submissions.total_tokens AS total_tokens,
        CAST(submissions.total_cost AS DECIMAL(18,4)) AS total_cost,
        RANK() OVER (ORDER BY ${primaryOrderColumn} DESC) AS rank,
        SUM(submissions.total_tokens) OVER () AS total_tokens_all,
        SUM(CAST(submissions.total_cost AS DECIMAL(18,4))) OVER () AS total_cost_all,
        COUNT(*) OVER () AS unique_users_all
      FROM submissions
      INNER JOIN users ON submissions.user_id = users.id
    ),
    filtered AS (
      SELECT ranked.*, COUNT(*) OVER () AS filtered_users
      FROM ranked
      WHERE ${searchCondition}
    ),
    paged AS (
      SELECT *
      FROM filtered
      ORDER BY rank ASC, ${secondaryOrderColumn} DESC, LOWER(username) ASC, user_id ASC
      LIMIT ${limit}
      OFFSET ${offset}
    )
    SELECT
      COALESCE(
        (
          SELECT json_agg(
            json_build_object(
              'rank', rank,
              'userId', user_id,
              'username', username,
              'displayName', display_name,
              'avatarUrl', avatar_url,
              'totalTokens', total_tokens,
              'totalCost', total_cost
            )
            ORDER BY rank ASC, ${secondaryOrderColumn} DESC, LOWER(username) ASC, user_id ASC
          )
          FROM paged
        ),
        '[]'::json
      ) AS users,
      COALESCE((SELECT filtered_users FROM filtered LIMIT 1), 0)::int AS "totalUsers",
      COALESCE((SELECT total_tokens_all FROM ranked LIMIT 1), 0) AS "totalTokens",
      COALESCE((SELECT total_cost_all FROM ranked LIMIT 1), 0) AS "totalCost",
      COALESCE((SELECT unique_users_all FROM ranked LIMIT 1), 0)::int AS "uniqueUsers"
  `);

  return buildLeaderboardData(result[0], page, limit, "all", sortBy);
}

async function fetchLeaderboardData(
  period: Period,
  page: number,
  limit: number,
  sortBy: SortBy = "tokens",
  search: string = "",
  customFrom?: string,
  customTo?: string
): Promise<LeaderboardData> {
  if (period !== "all") {
    return fetchPeriodLeaderboardData(
      period,
      page,
      limit,
      sortBy,
      search,
      customFrom,
      customTo
    );
  }

  return fetchAllTimeLeaderboardData(page, limit, sortBy, search);
}

export function getLeaderboardData(
  period: Period = "all",
  page: number = 1,
  limit: number = 50,
  sortBy: SortBy = "tokens",
  search: string = "",
  customFrom?: string,
  customTo?: string
): Promise<LeaderboardData> {
  const cacheKey = period === "custom"
    ? `leaderboard:custom:${customFrom}:${customTo}:${page}:${limit}:${sortBy}:${search}`
    : `leaderboard:${period}:${page}:${limit}:${sortBy}:${search}`;

  return unstable_cache(
    () => fetchLeaderboardData(period, page, limit, sortBy, search, customFrom, customTo),
    [cacheKey],
    {
      tags: ["leaderboard", `leaderboard:${period}`],
      revalidate: 60,
    }
  )();
}

// ============================================================================
// USER RANK
// ============================================================================

async function fetchPeriodUserRank(
  username: string,
  period: Exclude<Period, "all">,
  sortBy: SortBy,
  customFrom?: string,
  customTo?: string
): Promise<LeaderboardUser | null> {
  const dateRange = getPeriodDateRange(period, new Date(), customFrom, customTo);

  if (!dateRange) {
    return null;
  }

  const primaryOrderColumn = sortBy === "cost" ? sql`total_cost` : sql`total_tokens`;
  const secondaryOrderColumn = sortBy === "cost" ? sql`total_tokens` : sql`total_cost`;
  const results = await db.execute<RankedLeaderboardDbRow>(sql`
    WITH aggregated AS (
      SELECT
        submissions.user_id AS user_id,
        users.username AS username,
        users.display_name AS display_name,
        users.avatar_url AS avatar_url,
        SUM(daily_breakdown.tokens) AS total_tokens,
        SUM(CAST(daily_breakdown.cost AS DECIMAL(18,4))) AS total_cost
      FROM daily_breakdown
      INNER JOIN submissions ON daily_breakdown.submission_id = submissions.id
      INNER JOIN users ON submissions.user_id = users.id
      WHERE daily_breakdown.date >= ${dateRange.start}
        AND daily_breakdown.date <= ${dateRange.end}
      GROUP BY submissions.user_id, users.username, users.display_name, users.avatar_url
    ),
    ranked AS (
      SELECT
        aggregated.*,
        ROW_NUMBER() OVER (
          ORDER BY ${primaryOrderColumn} DESC, ${secondaryOrderColumn} DESC, LOWER(username) ASC, user_id ASC
        ) AS rank
      FROM aggregated
    )
    SELECT
      rank,
      user_id AS "userId",
      username,
      display_name AS "displayName",
      avatar_url AS "avatarUrl",
      total_tokens AS "totalTokens",
      total_cost AS "totalCost"
    FROM ranked
    WHERE LOWER(username) = LOWER(${username})
  `);

  const row = getSingleUsernameMatch(results, username);
  return row ? mapLeaderboardUser(row) : null;
}

async function fetchUserRank(
  username: string,
  period: Period,
  sortBy: SortBy,
  customFrom?: string,
  customTo?: string
): Promise<LeaderboardUser | null> {
  if (period !== "all") {
    return fetchPeriodUserRank(username, period, sortBy, customFrom, customTo);
  }

  const userResult = await db
    .select({ id: users.id, username: users.username, displayName: users.displayName, avatarUrl: users.avatarUrl })
    .from(users)
    .where(usernameEqualsIgnoreCase(username))
    .limit(USERNAME_LOOKUP_LIMIT);

  const user = getSingleUsernameMatch(userResult, username);

  if (!user) {
    return null;
  }

  const userStatsResult = await db
    .select({
      totalTokens: submissions.totalTokens,
      totalCost: sql<number>`CAST(${submissions.totalCost} AS DECIMAL(18,4))`.as("total_cost"),
    })
    .from(submissions)
    .where(eq(submissions.userId, user.id));

  if (!userStatsResult[0]) {
    return null;
  }

  const userStats = userStatsResult[0];
  const userTotalTokens = Number(userStats.totalTokens) || 0;
  const userTotalCost = Number(userStats.totalCost) || 0;

  const userCompareValue = sortBy === "cost"
    ? userTotalCost
    : userTotalTokens;
  const compareColumn = sortBy === "cost"
    ? sql`CAST(${submissions.totalCost} AS DECIMAL(18,4))`
    : submissions.totalTokens;

  const higherRankedResult = await db
    .select({
      count: sql<number>`COUNT(*)`.as("count"),
    })
    .from(submissions)
    .where(sql`${compareColumn} > ${userCompareValue}`);

  const rank = Number(higherRankedResult[0]?.count || 0) + 1;

  return {
    rank,
    userId: user.id,
    username: user.username,
    displayName: user.displayName,
    avatarUrl: user.avatarUrl,
    totalTokens: userTotalTokens,
    totalCost: userTotalCost,
  };
}

export function getUserRank(
  username: string,
  period: Period = "all",
  sortBy: SortBy = "tokens",
  customFrom?: string,
  customTo?: string
): Promise<LeaderboardUser | null> {
  const usernameCacheKey = normalizeUsernameCacheKey(username);
  const periodKey = period === "custom" ? `custom:${customFrom}:${customTo}` : period;

  return unstable_cache(
    () => fetchUserRank(username, period, sortBy, customFrom, customTo),
    [`user-rank:${usernameCacheKey}:${periodKey}:${sortBy}`],
    {
      tags: ["leaderboard", "user-rank", `user-rank:${usernameCacheKey}`],
      revalidate: 60,
    }
  )();
}
