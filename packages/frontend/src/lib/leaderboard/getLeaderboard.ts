import { unstable_cache } from "next/cache";
import { db, users, submissions, dailyBreakdown } from "@/lib/db";
import {
  USERNAME_LOOKUP_LIMIT,
  getSingleUsernameMatch,
  normalizeUsernameCacheKey,
  usernameEqualsIgnoreCase,
} from "@/lib/db/usernameLookup";
import { eq, desc, sql, and, gte, lte } from "drizzle-orm";
import { buildSubmissionFreshness } from "@/lib/submissionFreshness";
import type { LeaderboardData, LeaderboardUser, Period, SortBy } from "@/lib/leaderboard/types";

export type { LeaderboardData, LeaderboardUser, Period, SortBy } from "@/lib/leaderboard/types";

interface LeaderboardPeriodRow {
  userId: string;
  username: string;
  displayName: string | null;
  avatarUrl: string | null;
  tokens: number;
  cost: number;
  activeTimeMs: number | null;
  updatedAt: string;
  cliVersion: string | null;
  schemaVersion: number;
}

interface PeriodDateRange {
  start: string;
  end: string;
}

interface PeriodLeaderboardDbRow {
  userId: string;
  username: string;
  displayName: string | null;
  avatarUrl: string | null;
  tokens: number | string | null;
  cost: number | string | null;
  activeTimeMs: number | string | null;
  updatedAt: Date | string;
  cliVersion: string | null;
  schemaVersion: number | null;
}

interface AllTimeLeaderboardDbRow {
  userId: string;
  username: string;
  displayName: string | null;
  avatarUrl: string | null;
  totalTokens: number | string | null;
  totalCost: number | string | null;
  totalActiveTimeMs: number | string | null;
  submissionCount: number | string | null;
  lastSubmission: string;
  cliVersion: string | null;
  schemaVersion: number | null;
}

interface RankedLeaderboardDbRow extends AllTimeLeaderboardDbRow {
  rank: number | string | null;
}

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

function compareLeaderboardUsers(
  left: Omit<LeaderboardUser, "rank">,
  right: Omit<LeaderboardUser, "rank">,
  sortBy: SortBy
): number {
  if (sortBy === "time") {
    const leftTime = left.totalActiveTimeMs || 0;
    const rightTime = right.totalActiveTimeMs || 0;
    const primary = rightTime - leftTime;
    if (primary !== 0) return primary;
    const secondary = right.totalTokens - left.totalTokens;
    if (secondary !== 0) return secondary;
    return left.username.localeCompare(right.username);
  }

  const primary = sortBy === "cost"
    ? right.totalCost - left.totalCost
    : right.totalTokens - left.totalTokens;

  if (primary !== 0) {
    return primary;
  }

  const secondary = sortBy === "cost"
    ? right.totalTokens - left.totalTokens
    : right.totalCost - left.totalCost;

  if (secondary !== 0) {
    return secondary;
  }

  return left.username.localeCompare(right.username);
}

function aggregatePeriodRows(
  rows: LeaderboardPeriodRow[],
  sortBy: SortBy
): Array<Omit<LeaderboardUser, "rank">> {
  const usersById = new Map<string, Omit<LeaderboardUser, "rank">>();

  for (const row of rows) {
    const existing = usersById.get(row.userId);

    if (existing) {
      existing.totalTokens += row.tokens;
      existing.totalCost += row.cost;
      if (row.activeTimeMs != null) {
        existing.totalActiveTimeMs = (existing.totalActiveTimeMs || 0) + row.activeTimeMs;
      }
      if (row.updatedAt > existing.lastSubmission) {
        existing.lastSubmission = row.updatedAt;
        existing.submissionFreshness = buildSubmissionFreshness({
          updatedAt: row.updatedAt,
          cliVersion: row.cliVersion,
          schemaVersion: row.schemaVersion,
        });
      }
      continue;
    }

    usersById.set(row.userId, {
      userId: row.userId,
      username: row.username,
      displayName: row.displayName,
      avatarUrl: row.avatarUrl,
      totalTokens: row.tokens,
      totalCost: row.cost,
      totalActiveTimeMs: row.activeTimeMs,
      submissionCount: null,
      lastSubmission: row.updatedAt,
      submissionFreshness: buildSubmissionFreshness({
        updatedAt: row.updatedAt,
        cliVersion: row.cliVersion,
        schemaVersion: row.schemaVersion,
      }),
    });
  }

  return Array.from(usersById.values()).sort((left, right) =>
    compareLeaderboardUsers(left, right, sortBy)
  );
}

function matchesLeaderboardSearch(
  user: Pick<LeaderboardUser, "username" | "displayName">,
  search: string
): boolean {
  if (!search) {
    return true;
  }

  const lowerSearch = search.toLowerCase();
  if (user.username.toLowerCase().includes(lowerSearch)) {
    return true;
  }
  if (user.displayName && user.displayName.toLowerCase().includes(lowerSearch)) {
    return true;
  }
  return false;
}

function buildPeriodLeaderboardData(
  rows: LeaderboardPeriodRow[],
  page: number,
  limit: number,
  period: Period,
  sortBy: SortBy = "tokens",
  search: string = ""
): LeaderboardData {
  const offset = (page - 1) * limit;
  const aggregatedUsers = aggregatePeriodRows(rows, sortBy);
  const rankedUsers = aggregatedUsers.map((user, index) => ({
    ...user,
    rank: index + 1,
  }));
  const filteredUsers = rankedUsers.filter((user) =>
    matchesLeaderboardSearch(user, search)
  );
  const pagedUsers = filteredUsers.slice(offset, offset + limit);

  return {
    users: pagedUsers,
    pagination: {
      page,
      limit,
      totalUsers: filteredUsers.length,
      totalPages: Math.ceil(filteredUsers.length / limit),
      hasNext: offset + limit < filteredUsers.length,
      hasPrev: page > 1,
    },
    stats: {
      totalTokens: aggregatedUsers.reduce((sum, user) => sum + user.totalTokens, 0),
      totalCost: aggregatedUsers.reduce((sum, user) => sum + user.totalCost, 0),
      totalActiveTimeMs: aggregatedUsers.reduce((sum, user) => sum + (user.totalActiveTimeMs || 0), 0) || null,
      // submitCount lives on the all-time submission row, so period-scoped submit totals are unavailable here.
      totalSubmissions: null,
      uniqueUsers: aggregatedUsers.length,
    },
    period,
    sortBy,
  };
}

function buildPeriodUserRank(
  rows: LeaderboardPeriodRow[],
  username: string,
  sortBy: SortBy = "tokens"
): LeaderboardUser | null {
  const aggregatedUsers = aggregatePeriodRows(rows, sortBy);
  const usernameCacheKey = normalizeUsernameCacheKey(username);
  const matchingUsers = aggregatedUsers.filter(
    (user) => normalizeUsernameCacheKey(user.username) === usernameCacheKey
  );
  const user = getSingleUsernameMatch(matchingUsers, username);

  if (!user) {
    return null;
  }

  return {
    ...user,
    rank: aggregatedUsers.indexOf(user) + 1,
  };
}

async function fetchPeriodLeaderboardRows(
  period: Exclude<Period, "all">,
  customFrom?: string,
  customTo?: string
): Promise<LeaderboardPeriodRow[]> {
  const dateRange = getPeriodDateRange(period, new Date(), customFrom, customTo);

  if (!dateRange) {
    return [];
  }

  const rows: PeriodLeaderboardDbRow[] = await db
    .select({
      userId: users.id,
      username: users.username,
      displayName: users.displayName,
      avatarUrl: users.avatarUrl,
      tokens: dailyBreakdown.tokens,
      cost: dailyBreakdown.cost,
      activeTimeMs: dailyBreakdown.activeTimeMs,
      updatedAt: submissions.updatedAt,
      cliVersion: submissions.cliVersion,
      schemaVersion: submissions.schemaVersion,
    })
    .from(dailyBreakdown)
    .innerJoin(submissions, eq(dailyBreakdown.submissionId, submissions.id))
    .innerJoin(users, eq(submissions.userId, users.id))
    .where(
      and(
        gte(dailyBreakdown.date, dateRange.start),
        lte(dailyBreakdown.date, dateRange.end)
      )
    );

  return rows.map((row: PeriodLeaderboardDbRow) => ({
    userId: row.userId,
    username: row.username,
    displayName: row.displayName,
    avatarUrl: row.avatarUrl,
    tokens: Number(row.tokens) || 0,
    cost: Number(row.cost) || 0,
    activeTimeMs: row.activeTimeMs != null ? Number(row.activeTimeMs) : null,
    updatedAt: row.updatedAt instanceof Date
      ? row.updatedAt.toISOString()
      : new Date(row.updatedAt).toISOString(),
    cliVersion: row.cliVersion,
    schemaVersion: Number(row.schemaVersion) || 0,
  }));
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
    const rows = await fetchPeriodLeaderboardRows(period, customFrom, customTo);
    return buildPeriodLeaderboardData(rows, page, limit, period, sortBy, search);
  }

  const offset = (page - 1) * limit;

  const orderByColumn = sortBy === "cost"
    ? sql`SUM(CAST(${submissions.totalCost} AS DECIMAL(12,4)))`
    : sortBy === "time"
    ? sql`COALESCE(SUM(${submissions.totalActiveTimeMs}), 0)`
    : sql`SUM(${submissions.totalTokens})`;
  const secondaryOrderByColumn = sortBy === "cost"
    ? sql`SUM(${submissions.totalTokens})`
    : sortBy === "time"
    ? sql`SUM(${submissions.totalTokens})`
    : sql`SUM(CAST(${submissions.totalCost} AS DECIMAL(12,4)))`;

  if (search) {
    // When searching, use a subquery to compute global ranks for ALL users,
    // then filter by username. This preserves each user's true rank.
    const rankedSubquery = db
      .select({
        rank: sql<number>`RANK() OVER (ORDER BY ${orderByColumn} DESC)`.as("rank"),
        userId: users.id,
        username: users.username,
        displayName: users.displayName,
        avatarUrl: users.avatarUrl,
        totalTokens: sql<number>`SUM(${submissions.totalTokens})`.as("total_tokens"),
        totalCost: sql<number>`SUM(CAST(${submissions.totalCost} AS DECIMAL(12,4)))`.as("total_cost"),
        totalActiveTimeMs: sql<number>`COALESCE(SUM(${submissions.totalActiveTimeMs}), 0)`.as("total_active_time_ms"),
        submissionCount: sql<number>`COALESCE(SUM(${submissions.submitCount}), 0)`.as("submission_count"),
        lastSubmission: sql<string>`MAX(${submissions.updatedAt})`.as("last_submission"),
        cliVersion: sql<string | null>`(
          SELECT s2.cli_version FROM submissions s2
          WHERE s2.user_id = ${users.id}
          ORDER BY s2.updated_at DESC LIMIT 1
        )`.as("cli_version"),
        schemaVersion: sql<number>`COALESCE((
          SELECT s2.schema_version FROM submissions s2
          WHERE s2.user_id = ${users.id}
          ORDER BY s2.updated_at DESC LIMIT 1
        ), 0)`.as("schema_version"),
      })
      .from(submissions)
      .innerJoin(users, eq(submissions.userId, users.id))
      .groupBy(users.id, users.username, users.displayName, users.avatarUrl)
      .as("ranked");
    const rankedSecondaryOrderByColumn = sortBy === "cost"
      ? rankedSubquery.totalTokens
      : sortBy === "time"
      ? rankedSubquery.totalTokens
      : rankedSubquery.totalCost;

    const escapedSearch = search.toLowerCase().replace(/[%_\\]/g, "\\$&");
    const searchPattern = `%${escapedSearch}%`;
    const results = await db
      .select()
      .from(rankedSubquery)
      .where(sql`(LOWER(${rankedSubquery.username}) LIKE ${searchPattern} OR LOWER(COALESCE(${rankedSubquery.displayName}, '')) LIKE ${searchPattern})`)
      .orderBy(
        sql`${rankedSubquery.rank} ASC`,
        sql`${rankedSecondaryOrderByColumn} DESC`,
        sql`LOWER(${rankedSubquery.username}) ASC`
      )
      .limit(limit)
      .offset(offset);

    // Count total matching users for pagination
    const countResult = await db
      .select({ count: sql<number>`COUNT(*)`.as("count") })
      .from(rankedSubquery)
      .where(sql`(LOWER(${rankedSubquery.username}) LIKE ${searchPattern} OR LOWER(COALESCE(${rankedSubquery.displayName}, '')) LIKE ${searchPattern})`);

    const totalUsers = Number(countResult[0]?.count) || 0;
    const totalPages = Math.ceil(totalUsers / limit);

    // Global stats remain unfiltered
    const globalStats = await db
      .select({
        totalTokens: sql<number>`SUM(${submissions.totalTokens})`,
        totalCost: sql<number>`SUM(CAST(${submissions.totalCost} AS DECIMAL(12,4)))`,
        totalSubmissions: sql<number>`COUNT(${submissions.id})`,
        uniqueUsers: sql<number>`COUNT(DISTINCT ${submissions.userId})`,
      })
      .from(submissions);

    return {
      users: (results as RankedLeaderboardDbRow[]).map((row) => ({
        rank: Number(row.rank),
        userId: row.userId,
        username: row.username,
        displayName: row.displayName,
        avatarUrl: row.avatarUrl,
        totalTokens: Number(row.totalTokens) || 0,
        totalCost: Number(row.totalCost) || 0,
        totalActiveTimeMs: Number((row as AllTimeLeaderboardDbRow).totalActiveTimeMs) || null,
        submissionCount: Number(row.submissionCount) || 0,
        lastSubmission: row.lastSubmission,
        submissionFreshness: buildSubmissionFreshness({
          updatedAt: row.lastSubmission,
          cliVersion: row.cliVersion,
          schemaVersion: row.schemaVersion,
        }),
      })),
      pagination: {
        page,
        limit,
        totalUsers,
        totalPages,
        hasNext: page < totalPages,
        hasPrev: page > 1,
      },
      stats: {
        totalTokens: Number(globalStats[0]?.totalTokens) || 0,
        totalCost: Number(globalStats[0]?.totalCost) || 0,
        totalActiveTimeMs: null,
        totalSubmissions: Number(globalStats[0]?.totalSubmissions) || 0,
        uniqueUsers: Number(globalStats[0]?.uniqueUsers) || 0,
      },
      period,
      sortBy,
    };
  }

  // Non-search path: competition rank with deterministic row ordering for ties.
  const leaderboardQuery = db
    .select({
      rank: sql<number>`RANK() OVER (ORDER BY ${orderByColumn} DESC)`.as("rank"),
      userId: users.id,
      username: users.username,
      displayName: users.displayName,
      avatarUrl: users.avatarUrl,
      totalTokens: sql<number>`SUM(${submissions.totalTokens})`.as("total_tokens"),
      totalCost: sql<number>`SUM(CAST(${submissions.totalCost} AS DECIMAL(12,4)))`.as("total_cost"),
      totalActiveTimeMs: sql<number>`COALESCE(SUM(${submissions.totalActiveTimeMs}), 0)`.as("total_active_time_ms"),
      submissionCount: sql<number>`COALESCE(SUM(${submissions.submitCount}), 0)`.as("submission_count"),
      lastSubmission: sql<string>`MAX(${submissions.updatedAt})`.as("last_submission"),
      cliVersion: sql<string | null>`(
        SELECT s2.cli_version FROM submissions s2
        WHERE s2.user_id = ${users.id}
        ORDER BY s2.updated_at DESC LIMIT 1
      )`.as("cli_version"),
      schemaVersion: sql<number>`COALESCE((
        SELECT s2.schema_version FROM submissions s2
        WHERE s2.user_id = ${users.id}
        ORDER BY s2.updated_at DESC LIMIT 1
      ), 0)`.as("schema_version"),
    })
    .from(submissions)
    .innerJoin(users, eq(submissions.userId, users.id))
    .groupBy(users.id, users.username, users.displayName, users.avatarUrl)
    .orderBy(
      desc(orderByColumn),
      desc(secondaryOrderByColumn),
      sql`LOWER(${users.username}) ASC`
    )
    .limit(limit)
    .offset(offset);

  const [results, globalStats] = await Promise.all([
    leaderboardQuery,
    db
      .select({
        totalTokens: sql<number>`SUM(${submissions.totalTokens})`,
        totalCost: sql<number>`SUM(CAST(${submissions.totalCost} AS DECIMAL(12,4)))`,
        totalSubmissions: sql<number>`COUNT(${submissions.id})`,
        uniqueUsers: sql<number>`COUNT(DISTINCT ${submissions.userId})`,
      })
      .from(submissions),
  ]);

  const totalUsers = Number(globalStats[0]?.uniqueUsers) || 0;
  const totalPages = Math.ceil(totalUsers / limit);

  return {
    users: (results as RankedLeaderboardDbRow[]).map((row) => ({
      rank: Number(row.rank),
      userId: row.userId,
      username: row.username,
      displayName: row.displayName,
      avatarUrl: row.avatarUrl,
      totalTokens: Number(row.totalTokens) || 0,
      totalCost: Number(row.totalCost) || 0,
      totalActiveTimeMs: Number(row.totalActiveTimeMs) || null,
      submissionCount: Number(row.submissionCount) || 0,
      lastSubmission: row.lastSubmission,
      submissionFreshness: buildSubmissionFreshness({
        updatedAt: row.lastSubmission,
        cliVersion: row.cliVersion,
        schemaVersion: row.schemaVersion,
      }),
    })),
    pagination: {
      page,
      limit,
      totalUsers,
      totalPages,
      hasNext: page < totalPages,
      hasPrev: page > 1,
    },
    stats: {
      totalTokens: Number(globalStats[0]?.totalTokens) || 0,
      totalCost: Number(globalStats[0]?.totalCost) || 0,
      totalActiveTimeMs: null,
      totalSubmissions: Number(globalStats[0]?.totalSubmissions) || 0,
      uniqueUsers: Number(globalStats[0]?.uniqueUsers) || 0,
    },
    period,
    sortBy,
  };
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

async function fetchUserRank(
  username: string,
  period: Period,
  sortBy: SortBy,
  customFrom?: string,
  customTo?: string
): Promise<LeaderboardUser | null> {
  if (period !== "all") {
    const rows = await fetchPeriodLeaderboardRows(period, customFrom, customTo);
    return buildPeriodUserRank(rows, username, sortBy);
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
      totalTokens: sql<number>`SUM(${submissions.totalTokens})`.as("total_tokens"),
      totalCost: sql<number>`SUM(CAST(${submissions.totalCost} AS DECIMAL(12,4)))`.as("total_cost"),
      totalActiveTimeMs: sql<number>`COALESCE(SUM(${submissions.totalActiveTimeMs}), 0)`.as("total_active_time_ms"),
      submissionCount: sql<number>`COALESCE(SUM(${submissions.submitCount}), 0)`.as("submission_count"),
      lastSubmission: sql<string>`MAX(${submissions.updatedAt})`.as("last_submission"),
      cliVersion: sql<string | null>`(
        SELECT s2.cli_version FROM submissions s2
        WHERE s2.user_id = ${user.id}
        ORDER BY s2.updated_at DESC LIMIT 1
      )`.as("cli_version"),
      schemaVersion: sql<number>`COALESCE((
        SELECT s2.schema_version FROM submissions s2
        WHERE s2.user_id = ${user.id}
        ORDER BY s2.updated_at DESC LIMIT 1
      ), 0)`.as("schema_version"),
    })
    .from(submissions)
    .where(eq(submissions.userId, user.id));

  if (!userStatsResult[0] || userStatsResult[0].totalTokens == null) {
    return null;
  }

  const userStats = userStatsResult[0];
  const userTotalTokens = Number(userStats.totalTokens);
  const userTotalCost = userStats.totalCost != null ? Number(userStats.totalCost) : 0;
  const userTotalActiveTimeMs = Number(userStats.totalActiveTimeMs) || 0;

  const userCompareValue = sortBy === "cost"
    ? userTotalCost
    : sortBy === "time"
    ? userTotalActiveTimeMs
    : userTotalTokens;
  const compareColumn = sortBy === "cost"
    ? sql`SUM(CAST(${submissions.totalCost} AS DECIMAL(12,4)))`
    : sortBy === "time"
    ? sql`COALESCE(SUM(${submissions.totalActiveTimeMs}), 0)`
    : sql`SUM(${submissions.totalTokens})`;

  const higherRankedResult = await db
    .select({
      count: sql<number>`COUNT(*)`.as("count"),
    })
    .from(
      db
        .select({
          userId: submissions.userId,
          total: compareColumn.as("total"),
        })
        .from(submissions)
        .groupBy(submissions.userId)
        .having(sql`${compareColumn} > ${userCompareValue}`)
        .as("higher_ranked")
    );

  const rank = Number(higherRankedResult[0]?.count || 0) + 1;

  return {
    rank,
    userId: user.id,
    username: user.username,
    displayName: user.displayName,
    avatarUrl: user.avatarUrl,
    totalTokens: userTotalTokens,
    totalCost: userTotalCost,
    totalActiveTimeMs: userTotalActiveTimeMs || null,
    submissionCount: Number(userStats.submissionCount) || 0,
    lastSubmission: userStats.lastSubmission,
    submissionFreshness: buildSubmissionFreshness({
      updatedAt: userStats.lastSubmission,
      cliVersion: userStats.cliVersion,
      schemaVersion: userStats.schemaVersion,
    }),
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
