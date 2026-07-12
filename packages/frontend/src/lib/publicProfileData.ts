import { NextResponse } from "next/server";
import { unstable_cache } from "next/cache";
import { db, users, submissions, dailyBreakdown } from "@/lib/db";
import { eq, desc, sql, and, gte, lte } from "drizzle-orm";
import {
  AmbiguousUsernameError,
  USERNAME_LOOKUP_LIMIT,
  getSingleUsernameMatch,
  normalizeUsernameCacheKey,
  usernameEqualsIgnoreCase,
} from "@/lib/db/usernameLookup";
import { buildSubmissionFreshness } from "@/lib/submissionFreshness";

const LEGACY_CLIENT_ALIASES: Record<string, string> = { kilocode: "kilo" };
function normalizeClientId(id: string): string {
  return LEGACY_CLIENT_ALIASES[id] ?? id;
}

const PROFILE_PERIODS = ["all", "week", "month"] as const;
type ProfilePeriod = (typeof PROFILE_PERIODS)[number];

export type PublicProfilePeriod = ProfilePeriod;

interface ProfilePeriodDateRange {
  start: string;
  end: string;
}

function toUtcDateString(date: Date): string {
  return date.toISOString().slice(0, 10);
}

function parseProfilePeriod(value: string | null): ProfilePeriod {
  return PROFILE_PERIODS.includes(value as ProfilePeriod)
    ? (value as ProfilePeriod)
    : "all";
}

function getProfilePeriodDateRange(
  period: ProfilePeriod,
  now: Date = new Date(),
): ProfilePeriodDateRange | null {
  if (period === "all") {
    return null;
  }

  const end = new Date(
    Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), now.getUTCDate()),
  );
  const start = new Date(end);
  start.setUTCDate(start.getUTCDate() - (period === "week" ? 6 : 29));

  return {
    start: toUtcDateString(start),
    end: toUtcDateString(end),
  };
}

function getRollingProfileDateRange(
  now: Date = new Date(),
): ProfilePeriodDateRange {
  const end = new Date(
    Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), now.getUTCDate()),
  );
  const targetYear = end.getUTCFullYear() - 1;
  const month = end.getUTCMonth();
  const lastValidDay = new Date(
    Date.UTC(targetYear, month + 1, 0),
  ).getUTCDate();
  const start = new Date(
    Date.UTC(targetYear, month, Math.min(end.getUTCDate(), lastValidDay)),
  );

  return {
    start: toUtcDateString(start),
    end: toUtcDateString(end),
  };
}

function serializeUpdatedAt(
  value: Date | string | null | undefined,
): string | null {
  if (!value) {
    return null;
  }

  return value instanceof Date
    ? value.toISOString()
    : new Date(value).toISOString();
}

export interface PublicProfileRouteParams {
  params: Promise<{ username: string }>;
}

export async function getPublicProfileResponse(
  request: Request,
  { params }: PublicProfileRouteParams,
) {
  try {
    const { username } = await params;
    const { searchParams } = new URL(request.url);
    const period = parseProfilePeriod(searchParams.get("period"));
    const periodRange = getProfilePeriodDateRange(period);
    const chartRange = periodRange ?? getRollingProfileDateRange();

    // Find user
    const matchingUsers = await db
      .select({
        id: users.id,
        username: users.username,
        displayName: users.displayName,
        avatarUrl: users.avatarUrl,
        createdAt: users.createdAt,
      })
      .from(users)
      .where(usernameEqualsIgnoreCase(username))
      .limit(USERNAME_LOOKUP_LIMIT);
    const user = getSingleUsernameMatch(matchingUsers, username);

    if (!user) {
      return NextResponse.json({ error: "User not found" }, { status: 404 });
    }

    if (username !== user.username) {
      const canonicalUrl = new URL(`/api/users/${user.username}`, request.url);
      if (period !== "all") {
        canonicalUrl.searchParams.set("period", period);
      }
      return NextResponse.redirect(canonicalUrl, 308);
    }

    const dailyBreakdownFilter = periodRange
      ? and(
          eq(submissions.userId, user.id),
          gte(dailyBreakdown.date, periodRange.start),
          lte(dailyBreakdown.date, periodRange.end),
        )
      : eq(submissions.userId, user.id);

    const [statsResult, latestSubmissionResult, rankResult, dailyData] =
      await Promise.all([
        db
          .select({
            totalTokens: sql<number>`COALESCE(SUM(${submissions.totalTokens}), 0)`,
            totalCost: sql<number>`COALESCE(SUM(CAST(${submissions.totalCost} AS DECIMAL(18,4))), 0)`,
            inputTokens: sql<number>`COALESCE(SUM(${submissions.inputTokens}), 0)`,
            outputTokens: sql<number>`COALESCE(SUM(${submissions.outputTokens}), 0)`,
            cacheReadTokens: sql<number>`COALESCE(SUM(${submissions.cacheReadTokens}), 0)`,
            cacheCreationTokens: sql<number>`COALESCE(SUM(${submissions.cacheCreationTokens}), 0)`,
            reasoningTokens: sql<number>`COALESCE(SUM(${submissions.reasoningTokens}), 0)`,
            submissionCount: sql<number>`COALESCE(MAX(${submissions.submitCount}), 0)`,
            earliestDate: sql<string>`MIN(${submissions.dateStart})`,
            latestDate: sql<string>`MAX(${submissions.dateEnd})`,
            totalActiveTimeMs: sql<number>`COALESCE(SUM(${submissions.totalActiveTimeMs}), 0)`,
            sessionCount: sql<number>`COALESCE(SUM(${submissions.sessionCount}), 0)`,
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
            mcpServers: submissions.mcpServers,
          })
          .from(submissions)
          .where(eq(submissions.userId, user.id))
          .orderBy(desc(submissions.updatedAt))
          .limit(1),

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
            activeTimeMs: dailyBreakdown.activeTimeMs,
            tokens: dailyBreakdown.tokens,
            cost: dailyBreakdown.cost,
            inputTokens: dailyBreakdown.inputTokens,
            outputTokens: dailyBreakdown.outputTokens,
            sourceBreakdown: dailyBreakdown.sourceBreakdown,
          })
          .from(dailyBreakdown)
          .innerJoin(
            submissions,
            eq(dailyBreakdown.submissionId, submissions.id),
          )
          .where(dailyBreakdownFilter)
          .orderBy(dailyBreakdown.date),
      ]);

    const [stats] = statsResult;
    const [latestSubmission] = latestSubmissionResult;
    const rank = (rankResult as unknown as { rank: number }[])[0]?.rank || null;

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

    const mergeClientModel = (
      client: ClientBreakdown,
      modelId: string,
      model: ModelData,
    ) => {
      if (!modelId) return;

      client.models ??= {};
      const existingModel = client.models[modelId];
      if (existingModel) {
        existingModel.tokens += model.tokens || 0;
        existingModel.cost += model.cost || 0;
        existingModel.input += model.input || 0;
        existingModel.output += model.output || 0;
        existingModel.cacheRead += model.cacheRead || 0;
        existingModel.cacheWrite += model.cacheWrite || 0;
        existingModel.reasoning += model.reasoning || 0;
        existingModel.messages += model.messages || 0;
        return;
      }

      client.models[modelId] = {
        tokens: model.tokens || 0,
        cost: model.cost || 0,
        input: model.input || 0,
        output: model.output || 0,
        cacheRead: model.cacheRead || 0,
        cacheWrite: model.cacheWrite || 0,
        reasoning: model.reasoning || 0,
        messages: model.messages || 0,
      };
    };

    const materializeLegacyClientModel = (client: ClientBreakdown) => {
      if (client.models && Object.keys(client.models).length > 0) return;
      if (client.modelId) mergeClientModel(client, client.modelId, client);
    };

    const mergeIncomingClientModels = (
      client: ClientBreakdown,
      incoming: ClientBreakdown,
    ) => {
      const nestedModels = Object.entries(incoming.models ?? {});
      if (nestedModels.length > 0) {
        for (const [modelId, model] of nestedModels) {
          mergeClientModel(client, modelId, model);
        }
      } else if (incoming.modelId) {
        mergeClientModel(client, incoming.modelId, incoming);
      }
    };

    const aggregatedDaily = new Map<
      string,
      {
        date: string;
        timestampMs: number | null;
        activeTimeMs: number;
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
        existing.activeTimeMs += Number(day.activeTimeMs) || 0;
        if (day.sourceBreakdown) {
          for (const [rawClient, data] of Object.entries(day.sourceBreakdown)) {
            const client = normalizeClientId(rawClient);
            const breakdown = data as ClientBreakdown;
            if (existing.clients[client]) {
              const existingClient = existing.clients[client];
              materializeLegacyClientModel(existingClient);
              existingClient.tokens += breakdown.tokens || 0;
              existingClient.cost += breakdown.cost || 0;
              existingClient.input += breakdown.input || 0;
              existingClient.output += breakdown.output || 0;
              existingClient.cacheRead += breakdown.cacheRead || 0;
              existingClient.cacheWrite += breakdown.cacheWrite || 0;
              existingClient.reasoning += breakdown.reasoning || 0;
              existingClient.messages += breakdown.messages || 0;
              mergeIncomingClientModels(existingClient, breakdown);
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
              for (const [modelId, modelData] of Object.entries(
                breakdown.models,
              )) {
                const existingModel = existing.models[modelId];
                if (existingModel) {
                  existingModel.tokens += modelData.tokens || 0;
                  existingModel.cost += modelData.cost || 0;
                } else {
                  existing.models[modelId] = {
                    tokens: modelData.tokens || 0,
                    cost: modelData.cost || 0,
                  };
                }
              }
            } else if (breakdown.modelId) {
              const existingModel = existing.models[breakdown.modelId];
              if (existingModel) {
                existingModel.tokens += breakdown.tokens || 0;
                existingModel.cost += breakdown.cost || 0;
              } else {
                existing.models[breakdown.modelId] = {
                  tokens: breakdown.tokens || 0,
                  cost: breakdown.cost || 0,
                };
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
              const existingClient = clients[client];
              materializeLegacyClientModel(existingClient);
              existingClient.tokens += breakdown.tokens || 0;
              existingClient.cost += breakdown.cost || 0;
              existingClient.input += breakdown.input || 0;
              existingClient.output += breakdown.output || 0;
              existingClient.cacheRead += breakdown.cacheRead || 0;
              existingClient.cacheWrite += breakdown.cacheWrite || 0;
              existingClient.reasoning += breakdown.reasoning || 0;
              existingClient.messages += breakdown.messages || 0;
              mergeIncomingClientModels(existingClient, breakdown);
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
              for (const [modelId, modelData] of Object.entries(
                breakdown.models,
              )) {
                const existingModel = models[modelId];
                if (existingModel) {
                  existingModel.tokens += modelData.tokens || 0;
                  existingModel.cost += modelData.cost || 0;
                } else {
                  models[modelId] = {
                    tokens: modelData.tokens || 0,
                    cost: modelData.cost || 0,
                  };
                }
              }
            } else if (breakdown.modelId) {
              const existingModel = models[breakdown.modelId];
              if (existingModel) {
                existingModel.tokens += breakdown.tokens || 0;
                existingModel.cost += breakdown.cost || 0;
              } else {
                models[breakdown.modelId] = {
                  tokens: breakdown.tokens || 0,
                  cost: breakdown.cost || 0,
                };
              }
            }
          }
        }
        aggregatedDaily.set(day.date, {
          date: day.date,
          timestampMs: day.timestampMs ?? null,
          activeTimeMs: Number(day.activeTimeMs) || 0,
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
    const scopedContributions = contributions.filter(
      ({ date }) => date >= chartRange.start && date <= chartRange.end,
    );
    const maxCost = Math.max(...contributions.map((c) => c.cost), 0);
    const periodTotals = scopedContributions.reduce(
      (totals, day) => {
        totals.totalTokens += day.tokens;
        totals.totalCost += day.cost;
        totals.inputTokens += day.inputTokens;
        totals.outputTokens += day.outputTokens;
        totals.totalActiveTimeMs += day.activeTimeMs;

        for (const clientData of Object.values(day.clients)) {
          totals.cacheReadTokens += clientData.cacheRead || 0;
          totals.cacheWriteTokens += clientData.cacheWrite || 0;
          totals.reasoningTokens += clientData.reasoning || 0;
        }

        return totals;
      },
      {
        totalTokens: 0,
        totalCost: 0,
        inputTokens: 0,
        outputTokens: 0,
        cacheReadTokens: 0,
        cacheWriteTokens: 0,
        reasoningTokens: 0,
        totalActiveTimeMs: 0,
      },
    );

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

    const activeDays = scopedContributions.filter((c) => c.tokens > 0).length;

    const modelUsageMap = new Map<string, { tokens: number; cost: number }>();
    for (const day of scopedContributions) {
      for (const [model, data] of Object.entries(day.models)) {
        const existing = modelUsageMap.get(model) || { tokens: 0, cost: 0 };
        existing.tokens += data.tokens;
        existing.cost += data.cost;
        modelUsageMap.set(model, existing);
      }
    }

    const totalModelCost = Array.from(modelUsageMap.entries())
      .filter(([model]) => model !== "<synthetic>")
      .reduce((sum, [, data]) => sum + data.cost, 0);
    const modelUsage = Array.from(modelUsageMap.entries())
      .filter(([model]) => model !== "<synthetic>")
      .map(([model, data]) => ({
        model,
        tokens: data.tokens,
        cost: data.cost,
        percentage: totalModelCost > 0 ? (data.cost / totalModelCost) * 100 : 0,
      }))
      .sort((a, b) => b.cost - a.cost || b.tokens - a.tokens);
    const periodClients = Array.from(
      new Set(scopedContributions.flatMap((day) => Object.keys(day.clients))),
    );
    const periodModels = Array.from(modelUsageMap.keys()).filter(
      (model) => model !== "<synthetic>",
    );
    const isPeriodFiltered = period !== "all";

    return NextResponse.json({
      user: {
        id: user.id,
        username: user.username,
        displayName: user.displayName,
        avatarUrl: user.avatarUrl,
        createdAt: user.createdAt,
        rank: isPeriodFiltered ? null : rank ? Number(rank) : null,
      },
      stats: {
        totalTokens: isPeriodFiltered
          ? periodTotals.totalTokens
          : Number(stats?.totalTokens) || 0,
        totalCost: isPeriodFiltered
          ? periodTotals.totalCost
          : Number(stats?.totalCost) || 0,
        inputTokens: isPeriodFiltered
          ? periodTotals.inputTokens
          : Number(stats?.inputTokens) || 0,
        outputTokens: isPeriodFiltered
          ? periodTotals.outputTokens
          : Number(stats?.outputTokens) || 0,
        cacheReadTokens: isPeriodFiltered
          ? periodTotals.cacheReadTokens
          : Number(stats?.cacheReadTokens) || 0,
        cacheWriteTokens: isPeriodFiltered
          ? periodTotals.cacheWriteTokens
          : Number(stats?.cacheCreationTokens) || 0,
        reasoningTokens: isPeriodFiltered
          ? periodTotals.reasoningTokens
          : Number(stats?.reasoningTokens) || 0,
        submissionCount: Number(stats?.submissionCount) || 0,
        activeDays,
        totalActiveTimeMs: isPeriodFiltered
          ? periodTotals.totalActiveTimeMs
          : Number(stats?.totalActiveTimeMs) || 0,
        // Session count is only stored at submission level, so hide it for rolling ranges.
        sessionCount: isPeriodFiltered ? 0 : Number(stats?.sessionCount) || 0,
      },
      dateRange: {
        start: periodRange?.start ?? stats?.earliestDate ?? null,
        end: periodRange?.end ?? stats?.latestDate ?? null,
      },
      chartRange,
      period,
      updatedAt: serializeUpdatedAt(latestSubmission?.updatedAt),
      submissionFreshness: buildSubmissionFreshness({
        updatedAt: latestSubmission?.updatedAt,
        cliVersion: latestSubmission?.cliVersion,
        schemaVersion: latestSubmission?.schemaVersion,
      }),
      clients: isPeriodFiltered
        ? periodClients
        : latestSubmission?.sourcesUsed || [],
      models: isPeriodFiltered
        ? periodModels
        : latestSubmission?.modelsUsed || [],
      mcpServers: latestSubmission?.mcpServers || [],
      modelUsage,
      contributions: graphContributions,
    });
  } catch (error) {
    if (error instanceof AmbiguousUsernameError) {
      return NextResponse.json(
        { error: "Username is ambiguous" },
        { status: 409 },
      );
    }

    console.error("Profile error:", error);
    return NextResponse.json(
      { error: "Failed to fetch profile" },
      { status: 500 },
    );
  }
}

export type PublicProfilePageResult =
  | { data: unknown; kind: "data" }
  | { kind: "redirect"; location: string }
  | { kind: "error"; status: number };

export async function loadPublicProfileForPage(
  username: string,
  period: PublicProfilePeriod,
): Promise<PublicProfilePageResult> {
  const load = unstable_cache(
    async (): Promise<PublicProfilePageResult> => {
      const query = period === "all" ? "" : `?period=${period}`;
      const response = await getPublicProfileResponse(
        new Request(
          `http://tokscale.internal/api/users/${encodeURIComponent(username)}${query}`,
        ),
        { params: Promise.resolve({ username }) },
      );

      if (response.status === 308) {
        return {
          kind: "redirect",
          location: response.headers.get("location") ?? "",
        };
      }

      if (!response.ok) {
        return { kind: "error", status: response.status };
      }

      return { data: await response.json(), kind: "data" };
    },
    ["public-profile-page", username, period],
    {
      revalidate: 60,
      tags: [`user:${normalizeUsernameCacheKey(username)}`],
    },
  );

  return load();
}
