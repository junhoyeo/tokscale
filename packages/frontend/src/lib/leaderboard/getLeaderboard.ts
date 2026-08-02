import { unstable_cache } from "next/cache";
import { db, users, submissions, dailyBreakdown } from "@/lib/db";
import {
  USERNAME_LOOKUP_LIMIT,
  getSingleUsernameMatch,
  normalizeUsernameCacheKey,
  usernameEqualsIgnoreCase,
} from "@/lib/db/usernameLookup";
import { eq, desc, sql, and, or, gte, lte } from "drizzle-orm";
import type { LeaderboardData, LeaderboardUser, Period, SortBy } from "@/lib/leaderboard/types";
import {
  escapeLikePattern,
  hasDirectives,
  parseSearchDirectives,
} from "@/lib/leaderboard/searchDirectives";
import {
  scopeBreakdownToDirectives,
  type PeriodSourceBreakdown,
} from "@/lib/leaderboard/sourceBreakdown";

export type { LeaderboardData, LeaderboardUser, Period, SortBy } from "@/lib/leaderboard/types";

/**
 * Restricts a query to users eligible for a leaderboard position.
 *
 * Applies to RANKINGS ONLY. The site-wide totals queries deliberately omit it,
 * so a hidden user still contributes to total tokens, total cost and unique
 * user counts — hiding withdraws someone from the competition, it does not
 * erase their usage. Their profile, badge and embeds are likewise unaffected.
 */
const RANKABLE_USER = eq(users.leaderboardHidden, false);

interface LeaderboardPeriodRow {
  userId: string;
  username: string;
  displayName: string | null;
  avatarUrl: string | null;
  tokens: number;
  cost: number;
  sourceBreakdown: PeriodSourceBreakdown | null;
  /**
   * Carried through so the period path can drop the user from the rankings
   * while still counting them in the period totals. Internal only — it must
   * never reach LeaderboardUser, which is serialized to the public API.
   */
  leaderboardHidden: boolean;
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
  sourceBreakdown: PeriodSourceBreakdown | null;
  leaderboardHidden: boolean;
}

interface AllTimeLeaderboardDbRow {
  userId: string;
  username: string;
  displayName: string | null;
  avatarUrl: string | null;
  totalTokens: number | string | null;
  totalCost: number | string | null;
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
      continue;
    }

    usersById.set(row.userId, {
      userId: row.userId,
      username: row.username,
      displayName: row.displayName,
      avatarUrl: row.avatarUrl,
      totalTokens: row.tokens,
      totalCost: row.cost,
    });
  }

  return Array.from(usersById.values()).sort((left, right) =>
    compareLeaderboardUsers(left, right, sortBy)
  );
}

function matchesLeaderboardSearch(
  user: Pick<LeaderboardUser, "username" | "displayName">,
  textSearch: string
): boolean {
  if (!textSearch) {
    return true;
  }

  const lowerSearch = textSearch.toLowerCase();
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
  const parsed = parseSearchDirectives(search);

  let filteredRows = rows;
  if (hasDirectives(parsed)) {
    filteredRows = rows.flatMap((row) => {
      const scoped = scopeBreakdownToDirectives(row.sourceBreakdown, parsed);
      return scoped ? [{ ...row, tokens: scoped.tokens, cost: scoped.cost }] : [];
    });
  }

  // Aggregated twice on purpose. `aggregatedUsers` includes hidden users and
  // is what the period totals are computed from; `visibleUsers` excludes them
  // and is what gets ranked. Filtering once, before the totals, would silently
  // shrink the headline numbers — the opposite of the intended behaviour.
  const aggregatedUsers = aggregatePeriodRows(filteredRows, sortBy);
  const visibleUsers = aggregatePeriodRows(
    filteredRows.filter((row) => !row.leaderboardHidden),
    sortBy
  );

  // Ranked over the visible set, so ranks stay dense (1,2,3…) instead of
  // leaving a gap where the hidden user used to sit.
  const rankedUsers = visibleUsers.map((user, index) => ({
    ...user,
    rank: index + 1,
  }));
  const textFilteredUsers = rankedUsers.filter((user) =>
    matchesLeaderboardSearch(user, parsed.text)
  );
  const pagedUsers = textFilteredUsers.slice(offset, offset + limit);

  return {
    users: pagedUsers,
    pagination: {
      page,
      limit,
      totalUsers: textFilteredUsers.length,
      totalPages: Math.ceil(textFilteredUsers.length / limit),
      hasNext: offset + limit < textFilteredUsers.length,
      hasPrev: page > 1,
    },
    stats: {
      totalTokens: aggregatedUsers.reduce((sum, user) => sum + user.totalTokens, 0),
      totalCost: aggregatedUsers.reduce((sum, user) => sum + user.totalCost, 0),
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
  // Ranked against the visible set only. A hidden user is therefore absent
  // here and reports no rank at all, rather than a rank that does not
  // correspond to any position on the leaderboard.
  const aggregatedUsers = aggregatePeriodRows(
    rows.filter((row) => !row.leaderboardHidden),
    sortBy
  );
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
      sourceBreakdown: dailyBreakdown.sourceBreakdown,
      leaderboardHidden: users.leaderboardHidden,
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
    sourceBreakdown: row.sourceBreakdown ?? null,
    // Deliberately NOT filtered in SQL: the period totals are derived from
    // these same rows, and hidden users still count toward totals.
    leaderboardHidden: row.leaderboardHidden === true,
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
  const parsed = parseSearchDirectives(search);

  const orderByColumn = sortBy === "cost"
    ? sql`SUM(CAST(${submissions.totalCost} AS DECIMAL(18,4)))`
    : sql`SUM(${submissions.totalTokens})`;
  const secondaryOrderByColumn = sortBy === "cost"
    ? sql`SUM(${submissions.totalTokens})`
    : sql`SUM(CAST(${submissions.totalCost} AS DECIMAL(18,4)))`;

  const clientConditions = parsed.clients.map((client) =>
    sql`EXISTS (SELECT 1 FROM unnest(${submissions.sourcesUsed}) AS s WHERE LOWER(s) LIKE ${`%${escapeLikePattern(client)}%`})`
  );
  const modelConditions = parsed.models.map((model) =>
    sql`EXISTS (SELECT 1 FROM unnest(${submissions.modelsUsed}) AS m WHERE LOWER(m) LIKE ${`%${escapeLikePattern(model)}%`})`
  );
  const directiveConditions = [
    clientConditions.length > 0 ? or(...clientConditions) : undefined,
    modelConditions.length > 0 ? or(...modelConditions) : undefined,
  ].filter((condition): condition is ReturnType<typeof sql> => condition !== undefined);

  const hasTextSearch = parsed.text.length > 0;
  const hasDirectiveFilters = directiveConditions.length > 0;

  if (hasTextSearch || hasDirectiveFilters) {
    const rankedSubquery = db
      .select({
        rank: sql<number>`RANK() OVER (ORDER BY ${orderByColumn} DESC)`.as("rank"),
        userId: users.id,
        username: users.username,
        displayName: users.displayName,
        avatarUrl: users.avatarUrl,
        totalTokens: sql<number>`SUM(${submissions.totalTokens})`.as("total_tokens"),
        totalCost: sql<number>`SUM(CAST(${submissions.totalCost} AS DECIMAL(18,4)))`.as("total_cost"),
      })
      .from(submissions)
      .innerJoin(users, eq(submissions.userId, users.id))
      // Inside the ranked subquery, so RANK() renumbers densely rather than
      // leaving a hole where a hidden user was.
      .where(
        hasDirectiveFilters
          ? and(RANKABLE_USER, ...directiveConditions)
          : RANKABLE_USER
      )
      .groupBy(users.id, users.username, users.displayName, users.avatarUrl)
      .as("ranked");
    const rankedSecondaryOrderByColumn = sortBy === "cost"
      ? rankedSubquery.totalTokens
      : rankedSubquery.totalCost;

    let textFilter: ReturnType<typeof sql> | undefined;
    if (hasTextSearch) {
      const escapedSearch = escapeLikePattern(parsed.text.toLowerCase());
      const searchPattern = `%${escapedSearch}%`;
      textFilter = sql`(LOWER(${rankedSubquery.username}) LIKE ${searchPattern} OR LOWER(COALESCE(${rankedSubquery.displayName}, '')) LIKE ${searchPattern})`;
    }

    const results = await db
      .select()
      .from(rankedSubquery)
      .where(textFilter)
      .orderBy(
        sql`${rankedSubquery.rank} ASC`,
        sql`${rankedSecondaryOrderByColumn} DESC`,
        sql`LOWER(${rankedSubquery.username}) ASC`
      )
      .limit(limit)
      .offset(offset);

    const countResult = await db
      .select({ count: sql<number>`COUNT(*)`.as("count") })
      .from(rankedSubquery)
      .where(textFilter);

    const totalUsers = Number(countResult[0]?.count) || 0;
    const totalPages = Math.ceil(totalUsers / limit);

    const globalStats = await db
      .select({
        totalTokens: sql<number>`SUM(${submissions.totalTokens})`,
        totalCost: sql<number>`SUM(CAST(${submissions.totalCost} AS DECIMAL(18,4)))`,
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
      totalCost: sql<number>`SUM(CAST(${submissions.totalCost} AS DECIMAL(18,4)))`.as("total_cost"),
    })
    .from(submissions)
    .innerJoin(users, eq(submissions.userId, users.id))
    .where(RANKABLE_USER)
    .groupBy(users.id, users.username, users.displayName, users.avatarUrl)
    .orderBy(
      desc(orderByColumn),
      desc(secondaryOrderByColumn),
      sql`LOWER(${users.username}) ASC`
    )
    .limit(limit)
    .offset(offset);

  const [results, globalStats, rankableCount] = await Promise.all([
    leaderboardQuery,
    // Unfiltered: site-wide totals count every submission, hidden or not.
    db
      .select({
        totalTokens: sql<number>`SUM(${submissions.totalTokens})`,
        totalCost: sql<number>`SUM(CAST(${submissions.totalCost} AS DECIMAL(18,4)))`,
        uniqueUsers: sql<number>`COUNT(DISTINCT ${submissions.userId})`,
      })
      .from(submissions),
    // Pagination must count only the rows this query can actually return.
    // Reusing globalStats.uniqueUsers here would over-report by the number of
    // hidden users and leave a trailing page that renders empty.
    db
      .select({ count: sql<number>`COUNT(DISTINCT ${submissions.userId})`.as("count") })
      .from(submissions)
      .innerJoin(users, eq(submissions.userId, users.id))
      .where(RANKABLE_USER),
  ]);

  const totalUsers = Number(rankableCount[0]?.count) || 0;
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

  if (!user) {
    return null;
  }

  // A hidden user holds no leaderboard position, so they report no rank at all
  // rather than a number that matches nothing on the board.
  if (user.leaderboardHidden) {
    return null;
  }

  const userStatsResult = await db
    .select({
      totalTokens: sql<number>`SUM(${submissions.totalTokens})`.as("total_tokens"),
      totalCost: sql<number>`SUM(CAST(${submissions.totalCost} AS DECIMAL(18,4)))`.as("total_cost"),
    })
    .from(submissions)
    .where(eq(submissions.userId, user.id));

  if (!userStatsResult[0] || userStatsResult[0].totalTokens == null) {
    return null;
  }

  const userStats = userStatsResult[0];
  const userTotalTokens = Number(userStats.totalTokens);
  const userTotalCost = userStats.totalCost != null ? Number(userStats.totalCost) : 0;

  const userCompareValue = sortBy === "cost"
    ? userTotalCost
    : userTotalTokens;
  const compareColumn = sortBy === "cost"
    ? sql`SUM(CAST(${submissions.totalCost} AS DECIMAL(18,4)))`
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
        // Hidden users hold no position, so they must not be counted as being
        // "above" anyone — otherwise every visible user's rank is inflated by
        // however many hidden accounts outrank them.
        .innerJoin(users, eq(submissions.userId, users.id))
        .where(RANKABLE_USER)
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
