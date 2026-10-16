import { unstable_cache } from "next/cache";
import { and, asc, desc, eq, gte, lte, sql } from "drizzle-orm";
import { db, dailyBreakdown, groupMembers, submissions, users } from "@/lib/db";
import type { LeaderboardUser, SortBy } from "@/lib/leaderboard/types";
import { getCalendarDateRange, type GroupPeriod } from "@/lib/reportingPeriods";
import { hasDirectives, parseSearchDirectives } from "@/lib/leaderboard/searchDirectives";
import {
  scopeBreakdownToDirectives,
  type PeriodSourceBreakdown,
} from "@/lib/leaderboard/sourceBreakdown";

interface GroupLeaderboardPeriodRow {
  userId: string;
  username: string;
  displayName: string | null;
  avatarUrl: string | null;
  role: string;
  tokens: number;
  cost: number;
  sourceBreakdown: PeriodSourceBreakdown | null;
}

interface GroupLeaderboardDbRow extends Record<string, unknown> {
  userId: string;
  username: string;
  displayName: string | null;
  avatarUrl: string | null;
  role: string;
  totalTokens: number | string | null;
  totalCost: number | string | null;
}

export interface GroupLeaderboardUser extends LeaderboardUser {
  role: string;
}

export interface GroupLeaderboardData {
  users: GroupLeaderboardUser[];
  pagination: {
    page: number;
    limit: number;
    totalUsers: number;
    totalPages: number;
    hasNext: boolean;
    hasPrev: boolean;
  };
  stats: {
    totalTokens: number;
    totalCost: number;
    activeUsers: number;
    totalMembers: number;
  };
  period: GroupPeriod;
  sortBy: SortBy;
  dateRange: { start: string; end: string } | null;
  timeZone: string;
  weekStartsOn: number;
}

type GroupRankingData = Omit<GroupLeaderboardData, "dateRange" | "timeZone" | "weekStartsOn">;

function compareGroupUsers(
  left: Omit<GroupLeaderboardUser, "rank">,
  right: Omit<GroupLeaderboardUser, "rank">,
  sortBy: SortBy
): number {
  const primary = sortBy === "cost"
    ? right.totalCost - left.totalCost
    : right.totalTokens - left.totalTokens;

  if (primary !== 0) return primary;

  const secondary = sortBy === "cost"
    ? right.totalTokens - left.totalTokens
    : right.totalCost - left.totalCost;

  if (secondary !== 0) return secondary;

  return left.username.localeCompare(right.username);
}

function matchesSearch(
  user: Pick<GroupLeaderboardUser, "username" | "displayName">,
  normalizedText: string
): boolean {
  return normalizedText.length === 0 ||
    user.username.toLowerCase().includes(normalizedText) ||
    (user.displayName?.toLowerCase().includes(normalizedText) ?? false);
}

function paginateRankedUsers(
  usersWithRanks: GroupLeaderboardUser[],
  page: number,
  limit: number,
  period: GroupPeriod,
  sortBy: SortBy,
  search: string,
  totalMembers: number
): GroupRankingData {
  const offset = (page - 1) * limit;
  const normalizedText = parseSearchDirectives(search).text.toLowerCase();
  const filteredUsers = usersWithRanks.filter((user) => matchesSearch(user, normalizedText));
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
      totalTokens: usersWithRanks.reduce((sum, user) => sum + user.totalTokens, 0),
      totalCost: usersWithRanks.reduce((sum, user) => sum + user.totalCost, 0),
      activeUsers: usersWithRanks.length,
      totalMembers,
    },
    period,
    sortBy,
  };
}

function buildPeriodGroupLeaderboardData(
  rows: GroupLeaderboardPeriodRow[],
  page: number,
  limit: number,
  period: GroupPeriod,
  sortBy: SortBy,
  search: string,
  totalMembers: number
): GroupRankingData {
  const parsed = parseSearchDirectives(search);

  let filteredRows = rows;
  if (hasDirectives(parsed)) {
    filteredRows = rows.flatMap((row) => {
      const scoped = scopeBreakdownToDirectives(row.sourceBreakdown, parsed);
      return scoped ? [{ ...row, tokens: scoped.tokens, cost: scoped.cost }] : [];
    });
  }

  const usersById = new Map<string, Omit<GroupLeaderboardUser, "rank">>();

  for (const row of filteredRows) {
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
      role: row.role,
      totalTokens: row.tokens,
      totalCost: row.cost,
    });
  }

  const rankedUsers = Array.from(usersById.values())
    .sort((left, right) => compareGroupUsers(left, right, sortBy))
    .map((user, index) => ({ ...user, rank: index + 1 }));

  return paginateRankedUsers(
    rankedUsers,
    page,
    limit,
    period,
    sortBy,
    search,
    totalMembers
  );
}

async function countGroupMembers(groupId: string): Promise<number> {
  const memberCount = await db
    .select({ count: sql<number>`CAST(COUNT(*) AS integer)`.as("count") })
    .from(groupMembers)
    .where(eq(groupMembers.groupId, groupId));

  return Number(memberCount[0]?.count) || 0;
}

async function fetchPeriodRows(
  groupId: string,
  dateRange: { start: string; end: string },
): Promise<GroupLeaderboardPeriodRow[]> {

  const rows = await db
    .select({
      userId: users.id,
      username: users.username,
      displayName: users.displayName,
      avatarUrl: users.avatarUrl,
      role: groupMembers.role,
      tokens: dailyBreakdown.tokens,
      cost: dailyBreakdown.cost,
      sourceBreakdown: dailyBreakdown.sourceBreakdown,
    })
    .from(dailyBreakdown)
    .innerJoin(submissions, eq(dailyBreakdown.submissionId, submissions.id))
    .innerJoin(users, eq(submissions.userId, users.id))
    .innerJoin(
      groupMembers,
      and(
        eq(groupMembers.userId, submissions.userId),
        eq(groupMembers.groupId, groupId)
      )
    )
    // A group board is still a ranking, so a site-wide hide applies here too —
    // otherwise a hidden account keeps topping every group it belongs to.
    .where(
      and(
        eq(users.leaderboardHidden, false),
        gte(dailyBreakdown.date, dateRange.start),
        lte(dailyBreakdown.date, dateRange.end)
      )
    );

  return rows.map((row) => ({
    userId: row.userId,
    username: row.username,
    displayName: row.displayName,
    avatarUrl: row.avatarUrl,
    role: row.role,
    tokens: Number(row.tokens) || 0,
    cost: Number(row.cost) || 0,
    sourceBreakdown: row.sourceBreakdown ?? null,
  }));
}

function escapeLeaderboardLike(value: string): string {
  return value.replace(/[!%_]/g, "!$&");
}

function likeAny(
  column: ReturnType<typeof sql>,
  values: string[]
): ReturnType<typeof sql> {
  if (values.length === 0) return sql`TRUE`;
  const patterns = values.map((value) => `%${escapeLeaderboardLike(value)}%`);
  return sql`(${sql.join(
    patterns.map((pattern) => sql`LOWER(${column}) LIKE ${pattern} ESCAPE '!'`),
    sql` OR `
  )})`;
}

function toRankedAllTimeRows(rows: GroupLeaderboardDbRow[]): GroupLeaderboardUser[] {
  return rows.map((row, index) => ({
    rank: index + 1,
    userId: row.userId,
    username: row.username,
    displayName: row.displayName,
    avatarUrl: row.avatarUrl,
    role: row.role,
    totalTokens: Number(row.totalTokens) || 0,
    totalCost: Number(row.totalCost) || 0,
  }));
}

// The submission-level source/model arrays only prove that a client or model
// occurred somewhere in a user's history. They cannot say how much it used or
// whether a requested client and model occurred together. Filtered all-time
// boards therefore aggregate the same per-client/per-model daily JSON used by
// period boards; rows from before that JSON existed deliberately contribute
// nothing rather than re-crediting the user's entire lifetime total.
async function fetchScopedAllTimeRows(
  groupId: string,
  sortBy: SortBy,
  clients: string[],
  models: string[]
): Promise<GroupLeaderboardUser[]> {
  const usesModels = models.length > 0;
  const selectedBreakdown = usesModels ? sql`model.value` : sql`client.value`;
  const clientCondition = likeAny(sql`client.key`, clients);
  const modelRows = usesModels
    ? sql`CROSS JOIN LATERAL jsonb_each(COALESCE(client.value->'models', '{}'::jsonb)) AS model(key, value)`
    : sql``;
  const modelCondition = usesModels
    ? sql`AND ${likeAny(sql`model.key`, models)}`
    : sql``;
  const primaryOrderBy = sortBy === "cost" ? sql`"totalCost"` : sql`"totalTokens"`;
  const secondaryOrderBy = sortBy === "cost" ? sql`"totalTokens"` : sql`"totalCost"`;

  const rows = await db.execute<GroupLeaderboardDbRow>(sql`
    SELECT
      u.id AS "userId",
      u.username,
      u.display_name AS "displayName",
      u.avatar_url AS "avatarUrl",
      gm.role,
      SUM(COALESCE((${selectedBreakdown}->>'tokens')::numeric, 0)) AS "totalTokens",
      SUM(COALESCE((${selectedBreakdown}->>'cost')::numeric, 0)) AS "totalCost"
    FROM group_members gm
    INNER JOIN users u ON gm.user_id = u.id
    INNER JOIN submissions s ON s.user_id = u.id
    INNER JOIN daily_breakdown d ON d.submission_id = s.id
    CROSS JOIN LATERAL jsonb_each(COALESCE(d.source_breakdown, '{}'::jsonb)) AS client(key, value)
    ${modelRows}
    WHERE gm.group_id = ${groupId}
      AND u.leaderboard_hidden = false
      AND ${clientCondition}
      ${modelCondition}
    GROUP BY u.id, u.username, u.display_name, u.avatar_url, gm.role
    ORDER BY ${primaryOrderBy} DESC, ${secondaryOrderBy} DESC, LOWER(u.username) ASC, u.id ASC
  `);

  return toRankedAllTimeRows(rows);
}

async function fetchAllTimeRows(groupId: string, sortBy: SortBy, search: string = ""): Promise<GroupLeaderboardUser[]> {
  const parsed = parseSearchDirectives(search);

  if (hasDirectives(parsed)) {
    return fetchScopedAllTimeRows(groupId, sortBy, parsed.clients, parsed.models);
  }

  const primaryOrderByColumn = sortBy === "cost"
    ? sql`SUM(CAST(${submissions.totalCost} AS DECIMAL(18,4)))`
    : sql`SUM(${submissions.totalTokens})`;
  const secondaryOrderByColumn = sortBy === "cost"
    ? sql`SUM(${submissions.totalTokens})`
    : sql`SUM(CAST(${submissions.totalCost} AS DECIMAL(18,4)))`;

  const rows = await db
    .select({
      userId: users.id,
      username: users.username,
      displayName: users.displayName,
      avatarUrl: users.avatarUrl,
      role: groupMembers.role,
      totalTokens: sql<number>`SUM(${submissions.totalTokens})`.as("total_tokens"),
      totalCost: sql<number>`SUM(CAST(${submissions.totalCost} AS DECIMAL(18,4)))`.as("total_cost"),
    })
    .from(submissions)
    .innerJoin(users, eq(submissions.userId, users.id))
    .innerJoin(
      groupMembers,
      and(
        eq(groupMembers.userId, submissions.userId),
        eq(groupMembers.groupId, groupId)
      )
    )
    .where(eq(users.leaderboardHidden, false))
    .groupBy(users.id, users.username, users.displayName, users.avatarUrl, groupMembers.role)
    .orderBy(
      desc(primaryOrderByColumn),
      desc(secondaryOrderByColumn),
      asc(users.username),
      asc(users.id)
    );

  return toRankedAllTimeRows(rows as GroupLeaderboardDbRow[]);
}

async function fetchGroupLeaderboardData(
  groupId: string,
  period: GroupPeriod,
  page: number,
  limit: number,
  sortBy: SortBy,
  search: string,
  dateRange: { start: string; end: string } | null,
): Promise<GroupRankingData> {
  const totalMembers = await countGroupMembers(groupId);

  if (dateRange) {
    const rows = await fetchPeriodRows(groupId, dateRange);
    return buildPeriodGroupLeaderboardData(rows, page, limit, period, sortBy, search, totalMembers);
  }

  const usersWithRanks = await fetchAllTimeRows(groupId, sortBy, search);

  return paginateRankedUsers(
    usersWithRanks,
    page,
    limit,
    period,
    sortBy,
    search,
    totalMembers
  );
}

export function getGroupLeaderboardData(
  groupId: string,
  period: GroupPeriod = "all",
  page: number = 1,
  limit: number = 50,
  sortBy: SortBy = "tokens",
  search: string = "",
  timeZone: string = "UTC",
  weekStartsOn: number = 1,
): Promise<GroupLeaderboardData> {
  const dateRange = period === "all" ? null : getCalendarDateRange(period, new Date(), timeZone, weekStartsOn);
  return unstable_cache(
    () => fetchGroupLeaderboardData(groupId, period, page, limit, sortBy, search, dateRange),
    [`group-leaderboard:${groupId}:${period}:${page}:${limit}:${sortBy}:${search}:${timeZone}:${weekStartsOn}:${dateRange?.start ?? ""}:${dateRange?.end ?? ""}`],
    {
      tags: [
        `group:${groupId}`,
        `group-leaderboard:${groupId}`,
        `group-leaderboard:${groupId}:${period}`,
      ],
      revalidate: 60,
    }
  )().then((data) => ({ ...data, dateRange, timeZone, weekStartsOn }));
}
