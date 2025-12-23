import { unstable_cache } from "next/cache";
import { db, users, submissions } from "@/lib/db";
import { eq, desc, sql, and, gte, lte } from "drizzle-orm";

export type Period = "all" | "month" | "week";

export interface LeaderboardUser {
  rank: number;
  userId: string;
  username: string;
  displayName: string | null;
  avatarUrl: string | null;
  totalTokens: number;
  totalCost: number;
  submissionCount: number;
  lastSubmission: string;
}

export interface LeaderboardData {
  users: LeaderboardUser[];
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
    totalSubmissions: number;
    uniqueUsers: number;
  };
  period: Period;
}

function getDateFilter(period: Period) {
  const now = new Date();

  if (period === "week") {
    const weekAgo = new Date(now);
    weekAgo.setDate(weekAgo.getDate() - 7);
    return and(
      gte(submissions.createdAt, weekAgo),
      lte(submissions.createdAt, now)
    );
  }
  
  if (period === "month") {
    const monthAgo = new Date(now);
    monthAgo.setMonth(monthAgo.getMonth() - 1);
    return and(
      gte(submissions.createdAt, monthAgo),
      lte(submissions.createdAt, now)
    );
  }
  
  return undefined;
}

async function fetchLeaderboardData(
  period: Period,
  page: number,
  limit: number
): Promise<LeaderboardData> {
  const offset = (page - 1) * limit;
  const dateFilter = getDateFilter(period);

  const leaderboardQuery = db
    .select({
      rank: sql<number>`ROW_NUMBER() OVER (ORDER BY SUM(${submissions.totalTokens}) DESC)`.as("rank"),
      userId: users.id,
      username: users.username,
      displayName: users.displayName,
      avatarUrl: users.avatarUrl,
      totalTokens: sql<number>`SUM(${submissions.totalTokens})`.as("total_tokens"),
      totalCost: sql<number>`SUM(CAST(${submissions.totalCost} AS DECIMAL(12,4)))`.as("total_cost"),
      submissionCount: sql<number>`COALESCE(SUM(${submissions.submitCount}), 0)`.as("submission_count"),
      lastSubmission: sql<string>`MAX(${submissions.createdAt})`.as("last_submission"),
    })
    .from(submissions)
    .innerJoin(users, eq(submissions.userId, users.id))
    .where(dateFilter)
    .groupBy(users.id, users.username, users.displayName, users.avatarUrl)
    .orderBy(desc(sql`SUM(${submissions.totalTokens})`))
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
      .from(submissions)
      .where(dateFilter),
  ]);

  const totalUsers = Number(globalStats[0]?.uniqueUsers) || 0;
  const totalPages = Math.ceil(totalUsers / limit);

  return {
    users: results.map((row, index) => ({
      rank: offset + index + 1,
      userId: row.userId,
      username: row.username,
      displayName: row.displayName,
      avatarUrl: row.avatarUrl,
      totalTokens: Number(row.totalTokens) || 0,
      totalCost: Number(row.totalCost) || 0,
      submissionCount: Number(row.submissionCount) || 0,
      lastSubmission: row.lastSubmission,
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
      totalSubmissions: Number(globalStats[0]?.totalSubmissions) || 0,
      uniqueUsers: Number(globalStats[0]?.uniqueUsers) || 0,
    },
    period,
  };
}

export function getLeaderboardData(
  period: Period = "all",
  page: number = 1,
  limit: number = 50
): Promise<LeaderboardData> {
  return unstable_cache(
    () => fetchLeaderboardData(period, page, limit),
    [`leaderboard:${period}:${page}:${limit}`],
    {
      tags: ["leaderboard", `leaderboard:${period}`],
      revalidate: 60,
    }
  )();
}
