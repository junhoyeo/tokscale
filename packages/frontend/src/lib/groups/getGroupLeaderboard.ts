import { unstable_cache } from "next/cache";
import { and, asc, desc, eq, gte, lte, sql } from "drizzle-orm";
import { db, dailyBreakdown, groupMembers, submissions, users } from "@/lib/db";
import { buildSubmissionFreshness } from "@/lib/submissionFreshness";
import type { LeaderboardUser, Period, SortBy } from "@/lib/leaderboard/types";

interface GroupLeaderboardPeriodRow {
  userId: string;
  username: string;
  displayName: string | null;
  avatarUrl: string | null;
  role: string;
  tokens: number;
  cost: number;
  updatedAt: string;
  cliVersion: string | null;
  schemaVersion: number;
}

interface GroupLeaderboardDbRow {
  userId: string;
  username: string;
  displayName: string | null;
  avatarUrl: string | null;
  role: string;
  totalTokens: number | string | null;
  totalCost: number | string | null;
  submissionCount: number | string | null;
  lastSubmission: string;
  cliVersion: string | null;
  schemaVersion: number | null;
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
    totalSubmissions: number | null;
    uniqueUsers: number;
    activeUsers: number;
    totalMembers: number;
  };
  period: Period;
  sortBy: SortBy;
}

function toUtcDateString(date: Date): string {
  return date.toISOString().slice(0, 10);
}

function getPeriodDateRange(period: Period, now: Date = new Date()) {
  if (period === "all") {
    return null;
  }

  const end = new Date(Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), now.getUTCDate()));
  if (period === "week") {
    const start = new Date(end);
    start.setUTCDate(start.getUTCDate() - 6);
    return { start: toUtcDateString(start), end: toUtcDateString(end) };
  }

  const start = new Date(Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), 1));
  return { start: toUtcDateString(start), end: toUtcDateString(end) };
}

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

function matchesSearch(user: Pick<GroupLeaderboardUser, "username">, search: string): boolean {
  return !search || user.username.toLowerCase().includes(search.toLowerCase());
}

function paginateRankedUsers(
  usersWithRanks: GroupLeaderboardUser[],
  page: number,
  limit: number,
  period: Period,
  sortBy: SortBy,
  search: string,
  totalMembers: number,
  totalSubmissions: number | null
): GroupLeaderboardData {
  const offset = (page - 1) * limit;
  const filteredUsers = usersWithRanks.filter((user) => matchesSearch(user, search));
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
      totalSubmissions,
      uniqueUsers: usersWithRanks.length,
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
  period: Period,
  sortBy: SortBy,
  search: string,
  totalMembers: number
): GroupLeaderboardData {
  const usersById = new Map<string, Omit<GroupLeaderboardUser, "rank">>();

  for (const row of rows) {
    const existing = usersById.get(row.userId);
    if (existing) {
      existing.totalTokens += row.tokens;
      existing.totalCost += row.cost;
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
      role: row.role,
      totalTokens: row.tokens,
      totalCost: row.cost,
      totalActiveTimeMs: null,
      submissionCount: null,
      lastSubmission: row.updatedAt,
      submissionFreshness: buildSubmissionFreshness({
        updatedAt: row.updatedAt,
        cliVersion: row.cliVersion,
        schemaVersion: row.schemaVersion,
      }),
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
    totalMembers,
    null
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
  period: Exclude<Period, "all">
): Promise<GroupLeaderboardPeriodRow[]> {
  const dateRange = getPeriodDateRange(period);
  if (!dateRange) return [];

  const rows = await db
    .select({
      userId: users.id,
      username: users.username,
      displayName: users.displayName,
      avatarUrl: users.avatarUrl,
      role: groupMembers.role,
      tokens: dailyBreakdown.tokens,
      cost: dailyBreakdown.cost,
      updatedAt: submissions.updatedAt,
      cliVersion: submissions.cliVersion,
      schemaVersion: submissions.schemaVersion,
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
    .where(and(gte(dailyBreakdown.date, dateRange.start), lte(dailyBreakdown.date, dateRange.end)));

  return rows.map((row) => ({
    userId: row.userId,
    username: row.username,
    displayName: row.displayName,
    avatarUrl: row.avatarUrl,
    role: row.role,
    tokens: Number(row.tokens) || 0,
    cost: Number(row.cost) || 0,
    updatedAt: row.updatedAt instanceof Date
      ? row.updatedAt.toISOString()
      : new Date(row.updatedAt).toISOString(),
    cliVersion: row.cliVersion,
    schemaVersion: Number(row.schemaVersion) || 0,
  }));
}

async function fetchAllTimeRows(groupId: string, sortBy: SortBy): Promise<GroupLeaderboardUser[]> {
  const primaryOrderByColumn = sortBy === "cost"
    ? sql`SUM(CAST(${submissions.totalCost} AS DECIMAL(12,4)))`
    : sql`SUM(${submissions.totalTokens})`;
  const secondaryOrderByColumn = sortBy === "cost"
    ? sql`SUM(${submissions.totalTokens})`
    : sql`SUM(CAST(${submissions.totalCost} AS DECIMAL(12,4)))`;

  const rows = await db
    .select({
      userId: users.id,
      username: users.username,
      displayName: users.displayName,
      avatarUrl: users.avatarUrl,
      role: groupMembers.role,
      totalTokens: sql<number>`SUM(${submissions.totalTokens})`.as("total_tokens"),
      totalCost: sql<number>`SUM(CAST(${submissions.totalCost} AS DECIMAL(12,4)))`.as("total_cost"),
      submissionCount: sql<number>`COALESCE(SUM(${submissions.submitCount}), 0)`.as("submission_count"),
      lastSubmission: sql<string>`MAX(${submissions.updatedAt})`.as("last_submission"),
      cliVersion: sql<string | null>`(
        SELECT s2.cli_version FROM submissions s2
        WHERE s2.user_id = ${users.id}
        ORDER BY s2.updated_at DESC, s2.id DESC LIMIT 1
      )`.as("cli_version"),
      schemaVersion: sql<number>`COALESCE((
        SELECT s2.schema_version FROM submissions s2
        WHERE s2.user_id = ${users.id}
        ORDER BY s2.updated_at DESC, s2.id DESC LIMIT 1
      ), 0)`.as("schema_version"),
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
    .groupBy(users.id, users.username, users.displayName, users.avatarUrl, groupMembers.role)
    .orderBy(
      desc(primaryOrderByColumn),
      desc(secondaryOrderByColumn),
      asc(users.username),
      asc(users.id)
    );

  return (rows as GroupLeaderboardDbRow[]).map((row, index) => ({
    rank: index + 1,
    userId: row.userId,
    username: row.username,
    displayName: row.displayName,
    avatarUrl: row.avatarUrl,
    role: row.role,
    totalTokens: Number(row.totalTokens) || 0,
    totalCost: Number(row.totalCost) || 0,
    totalActiveTimeMs: null,
    submissionCount: Number(row.submissionCount) || 0,
    lastSubmission: row.lastSubmission,
    submissionFreshness: buildSubmissionFreshness({
      updatedAt: row.lastSubmission,
      cliVersion: row.cliVersion,
      schemaVersion: row.schemaVersion,
    }),
  }));
}

async function fetchGroupLeaderboardData(
  groupId: string,
  period: Period,
  page: number,
  limit: number,
  sortBy: SortBy,
  search: string
): Promise<GroupLeaderboardData> {
  const totalMembers = await countGroupMembers(groupId);

  if (period !== "all") {
    const rows = await fetchPeriodRows(groupId, period);
    return buildPeriodGroupLeaderboardData(rows, page, limit, period, sortBy, search, totalMembers);
  }

  const usersWithRanks = await fetchAllTimeRows(groupId, sortBy);
  const totalSubmissions = usersWithRanks.reduce(
    (sum, user) => sum + (user.submissionCount ?? 0),
    0
  );

  return paginateRankedUsers(
    usersWithRanks,
    page,
    limit,
    period,
    sortBy,
    search,
    totalMembers,
    totalSubmissions
  );
}

export function getGroupLeaderboardData(
  groupId: string,
  period: Period = "all",
  page: number = 1,
  limit: number = 50,
  sortBy: SortBy = "tokens",
  search: string = ""
): Promise<GroupLeaderboardData> {
  return unstable_cache(
    () => fetchGroupLeaderboardData(groupId, period, page, limit, sortBy, search),
    [`group-leaderboard:${groupId}:${period}:${page}:${limit}:${sortBy}:${search}`],
    {
      tags: [
        `group:${groupId}`,
        `group-leaderboard:${groupId}`,
        `group-leaderboard:${groupId}:${period}`,
      ],
      revalidate: 60,
    }
  )();
}
