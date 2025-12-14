import { NextResponse } from "next/server";
import { db, users, submissions } from "@/lib/db";
import { eq, desc, sql, and, gte, lte } from "drizzle-orm";

export const revalidate = 60; // ISR: revalidate every 60 seconds

type Period = "all" | "month" | "week";

export async function GET(request: Request) {
  try {
    const { searchParams } = new URL(request.url);
    const period = (searchParams.get("period") || "all") as Period;
    const page = Math.max(1, parseInt(searchParams.get("page") || "1"));
    const limit = Math.min(100, Math.max(1, parseInt(searchParams.get("limit") || "50")));
    const offset = (page - 1) * limit;

    // Calculate date range for period filter
    let dateFilter = undefined;
    const now = new Date();

    if (period === "week") {
      const weekAgo = new Date(now);
      weekAgo.setDate(weekAgo.getDate() - 7);
      dateFilter = and(
        gte(submissions.createdAt, weekAgo),
        lte(submissions.createdAt, now)
      );
    } else if (period === "month") {
      const monthAgo = new Date(now);
      monthAgo.setMonth(monthAgo.getMonth() - 1);
      dateFilter = and(
        gte(submissions.createdAt, monthAgo),
        lte(submissions.createdAt, now)
      );
    }

    // Build query for aggregated user stats
    const leaderboardQuery = db
      .select({
        rank: sql<number>`ROW_NUMBER() OVER (ORDER BY SUM(${submissions.totalTokens}) DESC)`.as("rank"),
        userId: users.id,
        username: users.username,
        displayName: users.displayName,
        avatarUrl: users.avatarUrl,
        totalTokens: sql<number>`SUM(${submissions.totalTokens})`.as("total_tokens"),
        totalCost: sql<number>`SUM(CAST(${submissions.totalCost} AS DECIMAL(12,4)))`.as("total_cost"),
        submissionCount: sql<number>`COUNT(${submissions.id})`.as("submission_count"),
        lastSubmission: sql<string>`MAX(${submissions.createdAt})`.as("last_submission"),
      })
      .from(submissions)
      .innerJoin(users, eq(submissions.userId, users.id))
      .where(dateFilter)
      .groupBy(users.id, users.username, users.displayName, users.avatarUrl)
      .orderBy(desc(sql`SUM(${submissions.totalTokens})`))
      .limit(limit)
      .offset(offset);

    const results = await leaderboardQuery;

    // Get total count for pagination
    const countQuery = await db
      .select({
        count: sql<number>`COUNT(DISTINCT ${submissions.userId})`,
      })
      .from(submissions)
      .where(dateFilter);

    const totalUsers = countQuery[0]?.count || 0;
    const totalPages = Math.ceil(totalUsers / limit);

    // Get global stats
    const globalStats = await db
      .select({
        totalTokens: sql<number>`SUM(${submissions.totalTokens})`,
        totalCost: sql<number>`SUM(CAST(${submissions.totalCost} AS DECIMAL(12,4)))`,
        totalSubmissions: sql<number>`COUNT(${submissions.id})`,
        uniqueUsers: sql<number>`COUNT(DISTINCT ${submissions.userId})`,
      })
      .from(submissions)
      .where(dateFilter);

    return NextResponse.json({
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
    });
  } catch (error) {
    console.error("Leaderboard error:", error);
    return NextResponse.json(
      { error: "Failed to fetch leaderboard" },
      { status: 500 }
    );
  }
}
