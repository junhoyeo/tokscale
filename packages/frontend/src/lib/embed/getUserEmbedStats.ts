import { unstable_cache } from "next/cache";
import { db, users, submissions } from "@/lib/db";
import { eq, sql } from "drizzle-orm";

export type EmbedSortBy = "tokens" | "cost";

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
      totalTokens: sql<number>`COALESCE(SUM(${submissions.totalTokens}), 0)`,
      totalCost: sql<number>`COALESCE(SUM(CAST(${submissions.totalCost} AS DECIMAL(12,4))), 0)`,
      submissionCount: sql<number>`COALESCE(SUM(${submissions.submitCount}), 0)`,
      updatedAt: sql<Date | null>`MAX(${submissions.updatedAt})`,
    })
    .from(users)
    .leftJoin(submissions, eq(submissions.userId, users.id))
    .where(eq(users.username, username))
    .groupBy(users.id, users.username, users.displayName, users.avatarUrl)
    .limit(1);

  if (!result) {
    return null;
  }

  let rank: number | null = null;

  const rankingValue = sortBy === "cost" ? Number(result.totalCost) || 0 : Number(result.totalTokens) || 0;

  if (rankingValue > 0) {
    const rankResult = await db.execute<{ rank: number }>(sql`
      WITH user_totals AS (
        SELECT
          user_id,
          SUM(total_tokens) AS total_tokens,
          SUM(CAST(total_cost AS DECIMAL(12,4))) AS total_cost
        FROM submissions
        GROUP BY user_id
      ),
      ranked AS (
        SELECT
          user_id,
          RANK() OVER (
            ORDER BY
              ${sortBy === "cost"
                ? sql`total_cost DESC, total_tokens DESC`
                : sql`total_tokens DESC, total_cost DESC`}
          ) AS rank
        FROM user_totals
      )
      SELECT rank FROM ranked WHERE user_id = ${result.id}
    `);

    const rawRank = (rankResult as unknown as Array<{ rank: number | string | null }>)[0]?.rank;
    const normalizedRank = rawRank == null ? null : Number(rawRank);
    rank = normalizedRank !== null && Number.isFinite(normalizedRank) ? normalizedRank : null;
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
      updatedAt: result.updatedAt instanceof Date
        ? result.updatedAt.toISOString()
        : result.updatedAt
        ? new Date(result.updatedAt).toISOString()
        : null,
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
