import { NextResponse } from "next/server";
import { db, users, submissions, dailyBreakdown } from "@/lib/db";
import { eq, desc, sql, and, gte } from "drizzle-orm";

export const revalidate = 60; // ISR: revalidate every 60 seconds

interface RouteParams {
  params: Promise<{ username: string }>;
}

export async function GET(_request: Request, { params }: RouteParams) {
  try {
    const { username } = await params;

    // Find user
    const [user] = await db
      .select({
        id: users.id,
        username: users.username,
        displayName: users.displayName,
        avatarUrl: users.avatarUrl,
        createdAt: users.createdAt,
      })
      .from(users)
      .where(eq(users.username, username))
      .limit(1);

    if (!user) {
      return NextResponse.json({ error: "User not found" }, { status: 404 });
    }

    // Get aggregate stats from all submissions
    const [stats] = await db
      .select({
        totalTokens: sql<number>`COALESCE(SUM(${submissions.totalTokens}), 0)`,
        totalCost: sql<number>`COALESCE(SUM(CAST(${submissions.totalCost} AS DECIMAL(12,4))), 0)`,
        inputTokens: sql<number>`COALESCE(SUM(${submissions.inputTokens}), 0)`,
        outputTokens: sql<number>`COALESCE(SUM(${submissions.outputTokens}), 0)`,
        cacheReadTokens: sql<number>`COALESCE(SUM(${submissions.cacheReadTokens}), 0)`,
        cacheCreationTokens: sql<number>`COALESCE(SUM(${submissions.cacheCreationTokens}), 0)`,
        submissionCount: sql<number>`COUNT(${submissions.id})`,
        earliestDate: sql<string>`MIN(${submissions.dateStart})`,
        latestDate: sql<string>`MAX(${submissions.dateEnd})`,
      })
      .from(submissions)
      .where(eq(submissions.userId, user.id));

    // Get latest submission for sources/models info
    const [latestSubmission] = await db
      .select({
        sourcesUsed: submissions.sourcesUsed,
        modelsUsed: submissions.modelsUsed,
      })
      .from(submissions)
      .where(eq(submissions.userId, user.id))
      .orderBy(desc(submissions.createdAt))
      .limit(1);

    // Get user rank
    const rankResult = await db.execute<{ rank: number }>(sql`
      WITH user_totals AS (
        SELECT 
          user_id,
          SUM(total_tokens) as total_tokens
        FROM submissions
        GROUP BY user_id
      ),
      ranked AS (
        SELECT 
          user_id,
          RANK() OVER (ORDER BY total_tokens DESC) as rank
        FROM user_totals
      )
      SELECT rank FROM ranked WHERE user_id = ${user.id}
    `);
    const rank = (rankResult as unknown as { rank: number }[])[0]?.rank || null;

    // Get daily breakdown data for graph (last 365 days)
    const oneYearAgo = new Date();
    oneYearAgo.setFullYear(oneYearAgo.getFullYear() - 1);

    const dailyData = await db
      .select({
        date: dailyBreakdown.date,
        tokens: dailyBreakdown.tokens,
        cost: dailyBreakdown.cost,
        inputTokens: dailyBreakdown.inputTokens,
        outputTokens: dailyBreakdown.outputTokens,
        sourceBreakdown: dailyBreakdown.sourceBreakdown,
        modelBreakdown: dailyBreakdown.modelBreakdown,
      })
      .from(dailyBreakdown)
      .innerJoin(submissions, eq(dailyBreakdown.submissionId, submissions.id))
      .where(
        and(
          eq(submissions.userId, user.id),
          gte(dailyBreakdown.date, oneYearAgo.toISOString().split("T")[0])
        )
      )
      .orderBy(dailyBreakdown.date);

    // Aggregate daily data (in case of overlapping submissions)
    const aggregatedDaily = new Map<
      string,
      {
        date: string;
        tokens: number;
        cost: number;
        inputTokens: number;
        outputTokens: number;
        sources: Record<string, number>;
        models: Record<string, number>;
      }
    >();

    for (const day of dailyData) {
      const existing = aggregatedDaily.get(day.date);
      if (existing) {
        existing.tokens += Number(day.tokens);
        existing.cost += Number(day.cost);
        existing.inputTokens += Number(day.inputTokens);
        existing.outputTokens += Number(day.outputTokens);
        // Merge breakdowns
        if (day.sourceBreakdown) {
          for (const [source, tokens] of Object.entries(day.sourceBreakdown)) {
            existing.sources[source] = (existing.sources[source] || 0) + (tokens as number);
          }
        }
        if (day.modelBreakdown) {
          for (const [model, tokens] of Object.entries(day.modelBreakdown)) {
            existing.models[model] = (existing.models[model] || 0) + (tokens as number);
          }
        }
      } else {
        aggregatedDaily.set(day.date, {
          date: day.date,
          tokens: Number(day.tokens),
          cost: Number(day.cost),
          inputTokens: Number(day.inputTokens),
          outputTokens: Number(day.outputTokens),
          sources: day.sourceBreakdown ? { ...(day.sourceBreakdown as Record<string, number>) } : {},
          models: day.modelBreakdown ? { ...(day.modelBreakdown as Record<string, number>) } : {},
        });
      }
    }

    // Calculate max cost for intensity
    const contributions = Array.from(aggregatedDaily.values());
    const maxCost = Math.max(...contributions.map((c) => c.cost), 0);

    // Build contribution graph data
    const graphContributions = contributions.map((day) => {
      const intensity =
        maxCost === 0
          ? 0
          : day.cost === 0
          ? 0
          : day.cost <= maxCost * 0.25
          ? 1
          : day.cost <= maxCost * 0.5
          ? 2
          : day.cost <= maxCost * 0.75
          ? 3
          : 4;

      return {
        date: day.date,
        totals: {
          tokens: day.tokens,
          cost: day.cost,
          messages: 0, // Not tracked in breakdown
        },
        intensity: intensity as 0 | 1 | 2 | 3 | 4,
        tokenBreakdown: {
          input: day.inputTokens,
          output: day.outputTokens,
          cacheRead: 0,
          cacheWrite: 0,
          reasoning: 0,
        },
        sources: Object.entries(day.sources).map(([source, tokens]) => ({
          source,
          modelId: "",
          tokens: {
            input: Math.floor((tokens as number) / 2),
            output: Math.floor((tokens as number) / 2),
            cacheRead: 0,
            cacheWrite: 0,
            reasoning: 0,
          },
          cost: 0,
          messages: 0,
        })),
      };
    });

    // Calculate active days
    const activeDays = contributions.filter((c) => c.tokens > 0).length;

    return NextResponse.json({
      user: {
        id: user.id,
        username: user.username,
        displayName: user.displayName,
        avatarUrl: user.avatarUrl,
        createdAt: user.createdAt,
        rank: rank ? Number(rank) : null,
      },
      stats: {
        totalTokens: Number(stats?.totalTokens) || 0,
        totalCost: Number(stats?.totalCost) || 0,
        inputTokens: Number(stats?.inputTokens) || 0,
        outputTokens: Number(stats?.outputTokens) || 0,
        cacheReadTokens: Number(stats?.cacheReadTokens) || 0,
        cacheCreationTokens: Number(stats?.cacheCreationTokens) || 0,
        submissionCount: Number(stats?.submissionCount) || 0,
        activeDays,
      },
      dateRange: {
        start: stats?.earliestDate || null,
        end: stats?.latestDate || null,
      },
      sources: latestSubmission?.sourcesUsed || [],
      models: latestSubmission?.modelsUsed || [],
      contributions: graphContributions,
    });
  } catch (error) {
    console.error("Profile error:", error);
    return NextResponse.json(
      { error: "Failed to fetch profile" },
      { status: 500 }
    );
  }
}
