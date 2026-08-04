import { unstable_cache } from "next/cache";
import { db, users } from "@/lib/db";
import {
  USERNAME_LOOKUP_LIMIT,
  getSingleUsernameMatch,
  normalizeUsernameCacheKey,
  usernameEqualsIgnoreCase,
} from "@/lib/db/usernameLookup";
import { sql } from "drizzle-orm";
import type {
  LeaderboardData,
  LeaderboardUser,
  Period,
  SortBy,
} from "@/lib/leaderboard/types";
import {
  hasDirectives,
  parseSearchDirectives,
} from "@/lib/leaderboard/searchDirectives";

export type {
  LeaderboardData,
  LeaderboardUser,
  Period,
  SortBy,
} from "@/lib/leaderboard/types";

interface PeriodDateRange {
  start: string;
  end: string;
}

type LeaderboardQueryResult = {
  users: unknown;
  totalUsers: number | string | null;
  totalTokens: number | string | null;
  totalCost: number | string | null;
  uniqueUsers: number | string | null;
};

type RankedLeaderboardDbRow = {
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
  customTo?: string,
): PeriodDateRange | null {
  if (period === "all") return null;
  if (period === "custom") {
    return customFrom && customTo ? { start: customFrom, end: customTo } : null;
  }

  const end = new Date(
    Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), now.getUTCDate()),
  );
  if (period === "week") {
    const start = new Date(end);
    start.setUTCDate(start.getUTCDate() - 6);
    return { start: toUtcDateString(start), end: toUtcDateString(end) };
  }
  if (period === "last-month") {
    const lastMonthEnd = new Date(
      Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), 0),
    );
    const lastMonthStart = new Date(
      Date.UTC(lastMonthEnd.getUTCFullYear(), lastMonthEnd.getUTCMonth(), 1),
    );
    return {
      start: toUtcDateString(lastMonthStart),
      end: toUtcDateString(lastMonthEnd),
    };
  }
  return {
    start: toUtcDateString(
      new Date(Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), 1)),
    ),
    end: toUtcDateString(end),
  };
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
    } catch (error) {
      throw new Error("Leaderboard query returned malformed users JSON", {
        cause: error,
      });
    }
  }
  if (!Array.isArray(rows)) {
    throw new Error("Leaderboard query returned a non-array users payload");
  }
  return rows.map((row) => {
    if (!row || typeof row !== "object") {
      throw new Error("Leaderboard query returned an invalid user row");
    }
    return mapLeaderboardUser(row as RankedLeaderboardDbRow);
  });
}

function buildLeaderboardData(
  row: LeaderboardQueryResult | undefined,
  page: number,
  limit: number,
  period: Period,
  sortBy: SortBy,
): LeaderboardData {
  if (!row) {
    return {
      users: [],
      pagination: {
        page,
        limit,
        totalUsers: 0,
        totalPages: 0,
        hasNext: false,
        hasPrev: page > 1,
      },
      stats: { totalTokens: 0, totalCost: 0, uniqueUsers: 0 },
      period,
      sortBy,
    };
  }
  const totalUsers = Number(row.totalUsers) || 0;
  const totalPages = Math.ceil(totalUsers / limit);
  return {
    users: parseLeaderboardUsers(row.users),
    pagination: {
      page,
      limit,
      totalUsers,
      totalPages,
      hasNext: page < totalPages,
      hasPrev: page > 1,
    },
    stats: {
      totalTokens: Number(row.totalTokens) || 0,
      totalCost: Number(row.totalCost) || 0,
      uniqueUsers: Number(row.uniqueUsers) || 0,
    },
    period,
    sortBy,
  };
}

function likeAny(
  column: ReturnType<typeof sql>,
  values: string[],
): ReturnType<typeof sql> {
  if (values.length === 0) return sql`TRUE`;
  const patterns = values.map((value) => `%${escapeLeaderboardLike(value)}%`);
  return sql`(${sql.join(
    patterns.map((pattern) => sql`LOWER(${column}) LIKE ${pattern} ESCAPE '!'`),
    sql` OR `,
  )})`;
}

function userTextCondition(text: string): ReturnType<typeof sql> {
  if (!text) return sql`TRUE`;
  const pattern = `%${escapeLeaderboardLike(text.toLowerCase())}%`;
  return sql`(LOWER(username) LIKE ${pattern} ESCAPE '!' OR LOWER(COALESCE(display_name, '')) LIKE ${pattern} ESCAPE '!')`;
}

function escapeLeaderboardLike(value: string): string {
  return value.replace(/[!%_]/g, "!$&");
}

function resultQuery(
  base: ReturnType<typeof sql>,
  page: number,
  limit: number,
  sortBy: SortBy,
  text: string,
  sequentialRanks: boolean,
  exactUsername?: string,
): ReturnType<typeof sql> {
  const offset = (page - 1) * limit;
  const primary = sortBy === "cost" ? sql`total_cost` : sql`total_tokens`;
  const secondary = sortBy === "cost" ? sql`total_tokens` : sql`total_cost`;
  const search = userTextCondition(text);
  return sql`
    WITH aggregated AS (${base}),
    rankable AS (
      SELECT aggregated.*, ${sequentialRanks ? sql`ROW_NUMBER()` : sql`RANK()`} OVER (ORDER BY ${primary} DESC) AS rank
      FROM aggregated
      WHERE leaderboard_hidden = false
    ),
    filtered AS (
      SELECT rankable.*, COUNT(*) OVER () AS filtered_users
      FROM rankable
      WHERE ${exactUsername ? sql`LOWER(username) = LOWER(${exactUsername})` : search}
    ),
    paged AS (
      SELECT * FROM filtered
      ORDER BY rank ASC, ${secondary} DESC, LOWER(username) ASC, user_id ASC
      LIMIT ${limit} OFFSET ${offset}
    )
    SELECT
      COALESCE((SELECT json_agg(json_build_object('rank', rank, 'userId', user_id, 'username', username, 'displayName', display_name, 'avatarUrl', avatar_url, 'totalTokens', total_tokens, 'totalCost', total_cost) ORDER BY rank ASC, ${secondary} DESC, LOWER(username) ASC, user_id ASC) FROM paged), '[]'::json) AS users,
      COALESCE((SELECT filtered_users FROM filtered LIMIT 1), 0)::int AS "totalUsers",
      COALESCE((SELECT SUM(total_tokens) FROM aggregated), 0) AS "totalTokens",
      COALESCE((SELECT SUM(total_cost) FROM aggregated), 0) AS "totalCost",
      COALESCE((SELECT COUNT(*) FROM aggregated), 0)::int AS "uniqueUsers"
  `;
}

async function fetchPeriodLeaderboardData(
  period: Exclude<Period, "all">,
  page: number,
  limit: number,
  sortBy: SortBy,
  search: string,
  customFrom?: string,
  customTo?: string,
): Promise<LeaderboardData> {
  const range = getPeriodDateRange(period, new Date(), customFrom, customTo);
  if (!range)
    return buildLeaderboardData(undefined, page, limit, period, sortBy);
  const parsed = parseSearchDirectives(search);
  let base: ReturnType<typeof sql>;

  if (!hasDirectives(parsed)) {
    base = sql`
      SELECT s.user_id, u.username, u.display_name, u.avatar_url, u.leaderboard_hidden,
        SUM(d.tokens) AS total_tokens, SUM(CAST(d.cost AS DECIMAL(18,4))) AS total_cost
      FROM daily_breakdown d
      INNER JOIN submissions s ON d.submission_id = s.id
      INNER JOIN users u ON s.user_id = u.id
      WHERE d.date >= ${range.start} AND d.date <= ${range.end}
      GROUP BY s.user_id, u.username, u.display_name, u.avatar_url, u.leaderboard_hidden
    `;
  } else {
    const clientMatch = likeAny(sql`client.key`, parsed.clients);
    const modelMatch = likeAny(sql`model.key`, parsed.models);
    const usesModels = parsed.models.length > 0;
    base = sql`
      SELECT s.user_id, u.username, u.display_name, u.avatar_url, u.leaderboard_hidden,
        SUM(CASE WHEN ${usesModels} THEN COALESCE((model.value->>'tokens')::numeric, 0) ELSE COALESCE((client.value->>'tokens')::numeric, 0) END) AS total_tokens,
        SUM(CASE WHEN ${usesModels} THEN COALESCE((model.value->>'cost')::numeric, 0) ELSE COALESCE((client.value->>'cost')::numeric, 0) END) AS total_cost
      FROM daily_breakdown d
      INNER JOIN submissions s ON d.submission_id = s.id
      INNER JOIN users u ON s.user_id = u.id
      CROSS JOIN LATERAL jsonb_each(COALESCE(d.source_breakdown, '{}'::jsonb)) AS client(key, value)
      LEFT JOIN LATERAL jsonb_each(COALESCE(client.value->'models', '{}'::jsonb)) AS model(key, value) ON ${usesModels}
      WHERE d.date >= ${range.start} AND d.date <= ${range.end}
        AND ${clientMatch} AND (${usesModels} = false OR ${modelMatch})
      GROUP BY s.user_id, u.username, u.display_name, u.avatar_url, u.leaderboard_hidden
    `;
  }
  const result = await db.execute<LeaderboardQueryResult>(
    resultQuery(base, page, limit, sortBy, parsed.text, true),
  );
  return buildLeaderboardData(result[0], page, limit, period, sortBy);
}

async function fetchAllTimeLeaderboardData(
  page: number,
  limit: number,
  sortBy: SortBy,
  search: string,
): Promise<LeaderboardData> {
  const parsed = parseSearchDirectives(search);
  const clientMatch = likeAny(sql`source`, parsed.clients);
  const modelMatch = likeAny(sql`model`, parsed.models);
  const sourceFilter = hasDirectives(parsed)
    ? sql`
    AND (${parsed.clients.length === 0} OR EXISTS (SELECT 1 FROM unnest(s.sources_used) AS source WHERE ${clientMatch}))
    AND (${parsed.models.length === 0} OR EXISTS (SELECT 1 FROM unnest(s.models_used) AS model WHERE ${modelMatch}))
  `
    : sql``;
  const base = sql`
    SELECT s.user_id, u.username, u.display_name, u.avatar_url, u.leaderboard_hidden,
      s.total_tokens, CAST(s.total_cost AS DECIMAL(18,4)) AS total_cost
    FROM submissions s
    INNER JOIN users u ON s.user_id = u.id
    WHERE TRUE ${sourceFilter}
  `;
  const result = await db.execute<LeaderboardQueryResult>(
    resultQuery(base, page, limit, sortBy, parsed.text, false),
  );
  return buildLeaderboardData(result[0], page, limit, "all", sortBy);
}

async function fetchLeaderboardData(
  period: Period,
  page: number,
  limit: number,
  sortBy: SortBy,
  search: string,
  customFrom?: string,
  customTo?: string,
): Promise<LeaderboardData> {
  return period === "all"
    ? fetchAllTimeLeaderboardData(page, limit, sortBy, search)
    : fetchPeriodLeaderboardData(
        period,
        page,
        limit,
        sortBy,
        search,
        customFrom,
        customTo,
      );
}

export function getLeaderboardData(
  period: Period = "all",
  page: number = 1,
  limit: number = 50,
  sortBy: SortBy = "tokens",
  search: string = "",
  customFrom?: string,
  customTo?: string,
): Promise<LeaderboardData> {
  const cacheKey =
    period === "custom"
      ? `leaderboard:custom:${customFrom}:${customTo}:${page}:${limit}:${sortBy}:${search}`
      : `leaderboard:${period}:${page}:${limit}:${sortBy}:${search}`;
  return unstable_cache(
    () =>
      fetchLeaderboardData(
        period,
        page,
        limit,
        sortBy,
        search,
        customFrom,
        customTo,
      ),
    [cacheKey],
    { tags: ["leaderboard", `leaderboard:${period}`], revalidate: 60 },
  )();
}

async function fetchUserRank(
  username: string,
  period: Period,
  sortBy: SortBy,
  customFrom?: string,
  customTo?: string,
): Promise<LeaderboardUser | null> {
  if (period !== "all") {
    const range = getPeriodDateRange(period, new Date(), customFrom, customTo);
    if (!range) return null;
    const base = sql`
      SELECT s.user_id, u.username, u.display_name, u.avatar_url, u.leaderboard_hidden,
        SUM(d.tokens) AS total_tokens, SUM(CAST(d.cost AS DECIMAL(18,4))) AS total_cost
      FROM daily_breakdown d
      INNER JOIN submissions s ON d.submission_id = s.id
      INNER JOIN users u ON s.user_id = u.id
      WHERE d.date >= ${range.start} AND d.date <= ${range.end}
      GROUP BY s.user_id, u.username, u.display_name, u.avatar_url, u.leaderboard_hidden
    `;
    const result = await db.execute<LeaderboardQueryResult>(
      resultQuery(base, 1, 1, sortBy, "", true, username),
    );
    return result[0]
      ? (parseLeaderboardUsers(result[0].users)[0] ?? null)
      : null;
  }
  const userResult = await db
    .select({
      id: users.id,
      username: users.username,
      displayName: users.displayName,
      avatarUrl: users.avatarUrl,
      leaderboardHidden: users.leaderboardHidden,
    })
    .from(users)
    .where(usernameEqualsIgnoreCase(username))
    .limit(USERNAME_LOOKUP_LIMIT);
  const user = getSingleUsernameMatch(userResult, username);
  if (!user || user.leaderboardHidden) return null;
  const base = sql`
        SELECT s.user_id, u.username, u.display_name, u.avatar_url, u.leaderboard_hidden,
          s.total_tokens, CAST(s.total_cost AS DECIMAL(18,4)) AS total_cost
        FROM submissions s INNER JOIN users u ON s.user_id = u.id
      `;
  const result = await db.execute<LeaderboardQueryResult>(
    resultQuery(base, 1, 1, sortBy, "", false, user.username),
  );
  return result[0] ? (parseLeaderboardUsers(result[0].users)[0] ?? null) : null;
}

export function getUserRank(
  username: string,
  period: Period = "all",
  sortBy: SortBy = "tokens",
  customFrom?: string,
  customTo?: string,
): Promise<LeaderboardUser | null> {
  const usernameCacheKey = normalizeUsernameCacheKey(username);
  const periodKey =
    period === "custom" ? `custom:${customFrom}:${customTo}` : period;
  return unstable_cache(
    () => fetchUserRank(username, period, sortBy, customFrom, customTo),
    [`user-rank:${usernameCacheKey}:${periodKey}:${sortBy}`],
    {
      tags: ["leaderboard", "user-rank", `user-rank:${usernameCacheKey}`],
      revalidate: 60,
    },
  )();
}
