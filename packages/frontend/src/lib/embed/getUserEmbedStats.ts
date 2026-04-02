import { unstable_cache } from "next/cache";
import { db, users, submissions, dailyBreakdown } from "@/lib/db";
import { eq, sql, and, gte } from "drizzle-orm";

export type EmbedSortBy = "tokens" | "cost";

export interface EmbedContributionDay {
  date: string;
  intensity: 0 | 1 | 2 | 3 | 4;
}

export interface UserEmbedStats {
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
    updatedAt: string | null;
  };
}

async function fetchUserEmbedStats(username: string, sortBy: EmbedSortBy): Promise<UserEmbedStats | null> {
  const [result] = await db
    .select({
      id: users.id,
      username: users.username,
      displayName: users.displayName,
      avatarUrl: users.avatarUrl,
      totalTokens: sql<number>`COALESCE(${submissions.totalTokens}, 0)`,
      totalCost: sql<number>`COALESCE(CAST(${submissions.totalCost} AS DECIMAL(12,4)), 0)`,
      submissionCount: sql<number>`COALESCE(${submissions.submitCount}, 0)`,
      updatedAt: submissions.updatedAt,
    })
    .from(users)
    .leftJoin(submissions, eq(submissions.userId, users.id))
    .where(eq(users.username, username))
    .limit(1);

  if (!result) {
    return null;
  }

  let rank: number | null = null;

  const rankingValue = sortBy === "cost" ? Number(result.totalCost) || 0 : Number(result.totalTokens) || 0;

  if (rankingValue > 0) {
    const rankResult = await db.execute<{ rank: number }>(sql`
      WITH ranked AS (
        SELECT
          user_id,
          RANK() OVER (
            ORDER BY
              ${sortBy === "cost"
                ? sql`CAST(total_cost AS DECIMAL(12,4)) DESC, total_tokens DESC`
                : sql`total_tokens DESC, CAST(total_cost AS DECIMAL(12,4)) DESC`}
          ) AS rank
        FROM submissions
      )
      SELECT rank FROM ranked WHERE user_id = ${result.id}
    `);

    rank = (rankResult as unknown as { rank: number }[])[0]?.rank || null;
  }

  return {
    user: {
      id: result.id,
      username: result.username,
      displayName: result.displayName,
      avatarUrl: result.avatarUrl,
    },
    stats: {
      totalTokens: Number(result.totalTokens) || 0,
      totalCost: Number(result.totalCost) || 0,
      submissionCount: Number(result.submissionCount) || 0,
      rank,
      updatedAt: result.updatedAt?.toISOString() || null,
    },
  };
}

export function getUserEmbedStats(username: string, sortBy: EmbedSortBy = "tokens"): Promise<UserEmbedStats | null> {
  return unstable_cache(
    () => fetchUserEmbedStats(username, sortBy),
    [`embed-user:${username}:${sortBy}`],
    {
      tags: [`user:${username}`, `embed-user:${username}`, `embed-user:${username}:${sortBy}`],
      revalidate: 60,
    }
  )();
}

async function fetchUserEmbedContributions(username: string): Promise<EmbedContributionDay[] | null> {
  const [user] = await db
    .select({ id: users.id })
    .from(users)
    .where(eq(users.username, username))
    .limit(1);

  if (!user) return null;

  const oneYearAgo = new Date();
  oneYearAgo.setFullYear(oneYearAgo.getFullYear() - 1);
  const cutoff = oneYearAgo.toISOString().split("T")[0];

  const rows = await db
    .select({ date: dailyBreakdown.date, cost: dailyBreakdown.cost })
    .from(dailyBreakdown)
    .innerJoin(submissions, eq(dailyBreakdown.submissionId, submissions.id))
    .where(and(eq(submissions.userId, user.id), gte(dailyBreakdown.date, cutoff)))
    .orderBy(dailyBreakdown.date);

  if (rows.length === 0) return [];

  const dayMap = new Map<string, number>();
  for (const row of rows) {
    dayMap.set(row.date, (dayMap.get(row.date) || 0) + (Number(row.cost) || 0));
  }

  const costs = Array.from(dayMap.values()).filter((c) => c > 0);
  const maxCost = Math.max(...costs, 0);

  return Array.from(dayMap.entries()).map(([date, cost]) => ({
    date,
    intensity: (
      maxCost === 0 ? 0 : cost === 0 ? 0 : cost <= maxCost * 0.25 ? 1 : cost <= maxCost * 0.5 ? 2 : cost <= maxCost * 0.75 ? 3 : 4
    ) as 0 | 1 | 2 | 3 | 4,
  }));
}

export function getUserEmbedContributions(username: string): Promise<EmbedContributionDay[] | null> {
  return unstable_cache(
    () => fetchUserEmbedContributions(username),
    [`embed-contrib:${username}`],
    {
      tags: [`user:${username}`, `embed-contrib:${username}`],
      revalidate: 60,
    }
  )();
}
