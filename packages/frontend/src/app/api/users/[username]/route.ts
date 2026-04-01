import { NextResponse } from "next/server";
import { db, users, submissions, dailyBreakdown } from "@/lib/db";
import { eq, desc, sql, and, gte } from "drizzle-orm";
import { buildSubmissionFreshness } from "@/lib/submissionFreshness";

const LEGACY_CLIENT_ALIASES: Record<string, string> = { kilocode: "kilo" };
function normalizeClientId(id: string): string {
  return LEGACY_CLIENT_ALIASES[id] ?? id;
}

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

    const oneYearAgo = new Date();
    oneYearAgo.setFullYear(oneYearAgo.getFullYear() - 1);

    const [statsResult, latestSubmissionResult, rankResult, dailyData] = await Promise.all([
      db
        .select({
          totalTokens: sql<number>`COALESCE(SUM(${submissions.totalTokens}), 0)`,
          totalCost: sql<number>`COALESCE(SUM(CAST(${submissions.totalCost} AS DECIMAL(12,4))), 0)`,
          inputTokens: sql<number>`COALESCE(SUM(${submissions.inputTokens}), 0)`,
          outputTokens: sql<number>`COALESCE(SUM(${submissions.outputTokens}), 0)`,
          cacheReadTokens: sql<number>`COALESCE(SUM(${submissions.cacheReadTokens}), 0)`,
          cacheCreationTokens: sql<number>`COALESCE(SUM(${submissions.cacheCreationTokens}), 0)`,
          reasoningTokens: sql<number>`COALESCE(SUM(${submissions.reasoningTokens}), 0)`,
          submissionCount: sql<number>`COALESCE(SUM(${submissions.submitCount}), 0)`,
          earliestDate: sql<string>`MIN(${submissions.dateStart})`,
          latestDate: sql<string>`MAX(${submissions.dateEnd})`,
        })
        .from(submissions)
        .where(eq(submissions.userId, user.id)),

      db
        .select({
          sourcesUsed: submissions.sourcesUsed,
          modelsUsed: submissions.modelsUsed,
          updatedAt: submissions.updatedAt,
          cliVersion: submissions.cliVersion,
          schemaVersion: submissions.schemaVersion,
        })
        .from(submissions)
        .where(eq(submissions.userId, user.id))
        .orderBy(desc(submissions.updatedAt)),

      db.execute<{ rank: number }>(sql`
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
      `),

      db
        .select({
          date: dailyBreakdown.date,
          timestampMs: dailyBreakdown.timestampMs,
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
        .orderBy(dailyBreakdown.date),
    ]);

    const [stats] = statsResult;
    const [latestSubmission] = latestSubmissionResult;
    const rank = (rankResult as unknown as { rank: number }[])[0]?.rank || null;
    const clients = new Set<string>();
    const models = new Set<string>();

    for (const submission of latestSubmissionResult) {
      for (const client of submission.sourcesUsed || []) {
        clients.add(normalizeClientId(client));
      }
      for (const model of submission.modelsUsed || []) {
        models.add(model);
      }
    }

    type ModelData = {
      tokens: number;
      cost: number;
      input: number;
      output: number;
      cacheRead: number;
      cacheWrite: number;
      reasoning: number;
      messages: number;
    };

    type ClientBreakdown = {
      tokens: number;
      cost: number;
      input: number;
      output: number;
      cacheRead: number;
      cacheWrite: number;
      reasoning: number;
      messages: number;
      models?: Record<string, ModelData>;
      modelId?: string;
    };

    const aggregatedDaily = new Map<
      string,
      {
        date: string;
        timestampMs: number | null;
        tokens: number;
        cost: number;
        inputTokens: number;
        outputTokens: number;
        clients: Record<string, ClientBreakdown>;
        models: Record<string, { tokens: number; cost: number }>;
      }
    >();

    for (const day of dailyData) {
      const existing = aggregatedDaily.get(day.date);
      if (existing) {
        if (day.timestampMs != null) {
          existing.timestampMs =
            existing.timestampMs != null
              ? Math.min(existing.timestampMs, day.timestampMs)
              : day.timestampMs;
        }
        existing.tokens += Number(day.tokens);
        existing.cost += Number(day.cost);
        existing.inputTokens += Number(day.inputTokens);
        existing.outputTokens += Number(day.outputTokens);
        if (day.sourceBreakdown) {
          for (const [rawClient, data] of Object.entries(day.sourceBreakdown)) {
            const client = normalizeClientId(rawClient);
            const breakdown = data as ClientBreakdown;
            if (existing.clients[client]) {
              existing.clients[client].tokens += breakdown.tokens || 0;
              existing.clients[client].cost += breakdown.cost || 0;
              existing.clients[client].input += breakdown.input || 0;
              existing.clients[client].output += breakdown.output || 0;
              existing.clients[client].cacheRead += breakdown.cacheRead || 0;
              existing.clients[client].cacheWrite += breakdown.cacheWrite || 0;
              existing.clients[client].reasoning += breakdown.reasoning || 0;
              existing.clients[client].messages += breakdown.messages || 0;
              if (breakdown.models) {
                existing.clients[client].models = existing.clients[client].models || {};
                for (const [modelId, modelData] of Object.entries(breakdown.models)) {
                  const existingModel = existing.clients[client].models![modelId];
                  if (existingModel) {
                    existingModel.tokens += modelData.tokens || 0;
                    existingModel.cost += modelData.cost || 0;
                    existingModel.input += modelData.input || 0;
                    existingModel.output += modelData.output || 0;
                    existingModel.cacheRead += modelData.cacheRead || 0;
                    existingModel.cacheWrite += modelData.cacheWrite || 0;
                    existingModel.reasoning += modelData.reasoning || 0;
                    existingModel.messages += modelData.messages || 0;
                  } else {
                    existing.clients[client].models![modelId] = {
                      tokens: modelData.tokens || 0,
                      cost: modelData.cost || 0,
                      input: modelData.input || 0,
                      output: modelData.output || 0,
                      cacheRead: modelData.cacheRead || 0,
                      cacheWrite: modelData.cacheWrite || 0,
                      reasoning: modelData.reasoning || 0,
                      messages: modelData.messages || 0,
                    };
                  }
                }
              }
            } else {
              existing.clients[client] = {
                tokens: breakdown.tokens || 0,
                cost: breakdown.cost || 0,
                input: breakdown.input || 0,
                output: breakdown.output || 0,
                cacheRead: breakdown.cacheRead || 0,
                cacheWrite: breakdown.cacheWrite || 0,
                reasoning: breakdown.reasoning || 0,
                messages: breakdown.messages || 0,
                models: breakdown.models,
                modelId: breakdown.modelId,
              };
            }
            if (breakdown.models) {
              for (const [modelId, modelData] of Object.entries(breakdown.models)) {
                const existingModel = existing.models[modelId];
                if (existingModel) {
                  existingModel.tokens += modelData.tokens || 0;
                  existingModel.cost += modelData.cost || 0;
                } else {
                  existing.models[modelId] = { tokens: modelData.tokens || 0, cost: modelData.cost || 0 };
                }
              }
            } else if (breakdown.modelId) {
              const existingModel = existing.models[breakdown.modelId];
              if (existingModel) {
                existingModel.tokens += breakdown.tokens || 0;
                existingModel.cost += breakdown.cost || 0;
              } else {
                existing.models[breakdown.modelId] = { tokens: breakdown.tokens || 0, cost: breakdown.cost || 0 };
              }
            }
          }
        }
       } else {
        const clients: Record<string, ClientBreakdown> = {};
        const models: Record<string, { tokens: number; cost: number }> = {};
        if (day.sourceBreakdown) {
          for (const [rawClient, data] of Object.entries(day.sourceBreakdown)) {
            const client = normalizeClientId(rawClient);
            const breakdown = data as ClientBreakdown;
            if (clients[client]) {
              // Merge when normalization creates duplicate keys (e.g. kilocode + kilo → kilo)
              clients[client].tokens += breakdown.tokens || 0;
              clients[client].cost += breakdown.cost || 0;
              clients[client].input += breakdown.input || 0;
              clients[client].output += breakdown.output || 0;
              clients[client].cacheRead += breakdown.cacheRead || 0;
              clients[client].cacheWrite += breakdown.cacheWrite || 0;
              clients[client].reasoning += breakdown.reasoning || 0;
              clients[client].messages += breakdown.messages || 0;
              if (breakdown.models) {
                clients[client].models = clients[client].models || {};
                for (const [modelId, modelData] of Object.entries(breakdown.models)) {
                  const existingModel = clients[client].models![modelId];
                  if (existingModel) {
                    existingModel.tokens += modelData.tokens || 0;
                    existingModel.cost += modelData.cost || 0;
                    existingModel.input += modelData.input || 0;
                    existingModel.output += modelData.output || 0;
                    existingModel.cacheRead += modelData.cacheRead || 0;
                    existingModel.cacheWrite += modelData.cacheWrite || 0;
                    existingModel.reasoning += modelData.reasoning || 0;
                    existingModel.messages += modelData.messages || 0;
                  } else {
                    clients[client].models![modelId] = {
                      tokens: modelData.tokens || 0,
                      cost: modelData.cost || 0,
                      input: modelData.input || 0,
                      output: modelData.output || 0,
                      cacheRead: modelData.cacheRead || 0,
                      cacheWrite: modelData.cacheWrite || 0,
                      reasoning: modelData.reasoning || 0,
                      messages: modelData.messages || 0,
                    };
                  }
                }
              }
            } else {
              clients[client] = {
                tokens: breakdown.tokens || 0,
                cost: breakdown.cost || 0,
                input: breakdown.input || 0,
                output: breakdown.output || 0,
                cacheRead: breakdown.cacheRead || 0,
                cacheWrite: breakdown.cacheWrite || 0,
                reasoning: breakdown.reasoning || 0,
                messages: breakdown.messages || 0,
                models: breakdown.models,
                modelId: breakdown.modelId,
              };
            }
            if (breakdown.models) {
              for (const [modelId, modelData] of Object.entries(breakdown.models)) {
                const existingModel = models[modelId];
                if (existingModel) {
                  existingModel.tokens += modelData.tokens || 0;
                  existingModel.cost += modelData.cost || 0;
                } else {
                  models[modelId] = { tokens: modelData.tokens || 0, cost: modelData.cost || 0 };
                }
              }
            } else if (breakdown.modelId) {
              const existingModel = models[breakdown.modelId];
              if (existingModel) {
                existingModel.tokens += breakdown.tokens || 0;
                existingModel.cost += breakdown.cost || 0;
              } else {
                models[breakdown.modelId] = { tokens: breakdown.tokens || 0, cost: breakdown.cost || 0 };
              }
            }
          }
        }
        aggregatedDaily.set(day.date, {
          date: day.date,
          timestampMs: day.timestampMs ?? null,
          tokens: Number(day.tokens),
          cost: Number(day.cost),
          inputTokens: Number(day.inputTokens),
          outputTokens: Number(day.outputTokens),
          clients,
          models,
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

      let dayCacheRead = 0;
      let dayCacheWrite = 0;
      let dayReasoning = 0;
      for (const clientData of Object.values(day.clients)) {
        dayCacheRead += clientData.cacheRead || 0;
        dayCacheWrite += clientData.cacheWrite || 0;
        dayReasoning += clientData.reasoning || 0;
      }

      return {
        date: day.date,
        timestampMs: day.timestampMs ?? null,
        totals: {
          tokens: day.tokens,
          cost: day.cost,
          messages: 0, // Not tracked in breakdown
        },
        intensity: intensity as 0 | 1 | 2 | 3 | 4,
        tokenBreakdown: {
          input: day.inputTokens,
          output: day.outputTokens,
          cacheRead: dayCacheRead,
          cacheWrite: dayCacheWrite,
          reasoning: dayReasoning,
        },
        clients: Object.entries(day.clients).map(([client, breakdown]) => ({
          client,
          modelId: breakdown.modelId || "",
          models: breakdown.models || {},
          tokens: {
            input: breakdown.input || 0,
            output: breakdown.output || 0,
            cacheRead: breakdown.cacheRead || 0,
            cacheWrite: breakdown.cacheWrite || 0,
            reasoning: breakdown.reasoning || 0,
          },
          cost: breakdown.cost || 0,
          messages: breakdown.messages || 0,
        })),
      };
    });

    const activeDays = contributions.filter((c) => c.tokens > 0).length;

    const modelUsageMap = new Map<string, { tokens: number; cost: number }>();
    for (const day of contributions) {
      for (const [model, data] of Object.entries(day.models)) {
        const existing = modelUsageMap.get(model) || { tokens: 0, cost: 0 };
        existing.tokens += data.tokens;
        existing.cost += data.cost;
        modelUsageMap.set(model, existing);
      }
    }

    const totalModelCost = Array.from(modelUsageMap.values()).reduce((sum, m) => sum + m.cost, 0);
    const modelUsage = Array.from(modelUsageMap.entries())
      .filter(([model]) => model !== "<synthetic>")
      .map(([model, data]) => ({
        model,
        tokens: data.tokens,
        cost: data.cost,
        percentage: totalModelCost > 0 ? (data.cost / totalModelCost) * 100 : 0,
      }))
      .sort((a, b) => b.cost - a.cost || b.tokens - a.tokens);

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
        cacheWriteTokens: Number(stats?.cacheCreationTokens) || 0,
        reasoningTokens: Number(stats?.reasoningTokens) || 0,
        submissionCount: Number(stats?.submissionCount) || 0,
        activeDays,
      },
      dateRange: {
        start: stats?.earliestDate || null,
        end: stats?.latestDate || null,
      },
      updatedAt: latestSubmission?.updatedAt?.toISOString() || null,
      submissionFreshness: buildSubmissionFreshness({
        updatedAt: latestSubmission?.updatedAt,
        cliVersion: latestSubmission?.cliVersion,
        schemaVersion: latestSubmission?.schemaVersion,
      }),
      clients: Array.from(clients).sort(),
      models: Array.from(models).sort(),
      modelUsage,
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
