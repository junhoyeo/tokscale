import { NextResponse } from "next/server";
import { unstable_cache } from "next/cache";
import { db, users, submissions, dailyBreakdown } from "@/lib/db";
import { eq, desc, sql, and, gte } from "drizzle-orm";
import {
  AmbiguousUsernameError,
  USERNAME_LOOKUP_LIMIT,
  getSingleUsernameMatch,
  normalizeUsernameCacheKey,
  usernameEqualsIgnoreCase,
} from "@/lib/db/usernameLookup";
import { buildSubmissionFreshness } from "@/lib/submissionFreshness";
import { calculateIntensity } from "@/lib/utils";

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

function getUtcToday(now: Date): Date {
  return new Date(
    Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), now.getUTCDate()),
  );
}

function parseProfilePeriod(value: string | null): ProfilePeriod {
  return PROFILE_PERIODS.includes(value as ProfilePeriod)
    ? (value as ProfilePeriod)
    : "all";
}

const DATE_KEY_PATTERN = /^\d{4}-\d{2}-\d{2}$/;

/** `YYYY-MM-DD` to its UTC midnight; null for anything that is not one. */
function parseDateKey(value: string | null | undefined): Date | null {
  if (!value || !DATE_KEY_PATTERN.test(value)) return null;
  const parsed = new Date(`${value}T00:00:00.000Z`);
  return Number.isNaN(parsed.getTime()) ? null : parsed;
}

/**
 * The day every profile window ends on: the later of UTC today and the newest
 * date the data itself carries.
 *
 * Contribution dates are calendar-day buckets computed by the CLI in the
 * *submitting* machine's local timezone, so a user ahead of UTC legitimately
 * reports a date that is still tomorrow here. Anchoring to the data keeps that
 * day inside every window for every viewer, wherever they are, and keeps the
 * range-scoped stats on the same window the chart draws. Validation caps
 * contribution dates at UTC today + 2 days, so the anchor can never run more
 * than two days past the present.
 */
function getProfileRangeAnchor(
  latestDate: string | null | undefined,
  now: Date,
): Date {
  const utcToday = getUtcToday(now);
  const latest = parseDateKey(latestDate);
  return latest && latest > utcToday ? latest : utcToday;
}

/** Trailing seven- or thirty-day window ending on `end`; null for lifetime. */
function getProfilePeriodDateRange(
  period: ProfilePeriod,
  end: Date,
): ProfilePeriodDateRange | null {
  if (period === "all") {
    return null;
  }

  const start = new Date(end);
  start.setUTCDate(start.getUTCDate() - (period === "week" ? 6 : 29));

  return {
    start: toUtcDateString(start),
    end: toUtcDateString(end),
  };
}

/** Rolling twelve-month window for the lifetime chart, ending on `end`. */
function getRollingProfileDateRange(end: Date): ProfilePeriodDateRange {
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
    // One clock reading for the whole request, so a UTC midnight crossed
    // mid-request cannot put the query and the window on different days.
    const now = new Date();
    // The window a period request draws ends on the anchor, which is only known
    // once the stats query returns. The anchor can only move *forward* of UTC
    // today, so a window measured back from UTC today starts on or before the
    // real one — fetching from there is a superset that `scopedContributions`
    // trims to the anchored window.
    const periodFetchStart = getProfilePeriodDateRange(
      period,
      getUtcToday(now),
    )?.start;

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

    // Deliberately unbounded above: `submissions.dateEnd` is
    // `MAX(dailyBreakdown.date)`, so the anchor already dominates every row this
    // user has and an upper bound could only ever drop the newest day of an
    // owner whose calendar runs ahead of UTC.
    const dailyBreakdownFilter = periodFetchStart
      ? and(
          eq(submissions.userId, user.id),
          gte(dailyBreakdown.date, periodFetchStart),
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
            // `submissions` is unique per user, so this SUM reads a single row.
            // That row's sessionCount is itself derived in the submit route by
            // summing PER-DEVICE counts, which is what makes the "all-time"
            // label defensible for multi-device users -- before that it was one
            // device's snapshot overwriting another's.
            //
            // Still an approximate historical maximum, not an exact count: each
            // device's stored value is a high-water mark, so a submit filtered
            // by --clients/--date can never lower it, and a re-sessionization
            // that legitimately merges two intervals cannot correct it down.
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
            hasBackfill: submissions.hasBackfill,
          })
          .from(submissions)
          .where(eq(submissions.userId, user.id))
          .orderBy(desc(submissions.updatedAt))
          .limit(1),

        // A finite rank needs the anchored profile range, which is only known
        // after the stats query returns the newest submitted date. Keep the
        // lifetime rank concurrent and defer only finite-period ranking.
        //
        // Shared RANK here on purpose: the leaderboard's all-time tab ranks
        // the same way, so tied users read the same number on both surfaces.
        period === "all"
          ? db.execute<{ rank: number }>(sql`
              WITH user_totals AS (
                SELECT
                  s.user_id,
                  SUM(s.total_tokens) as total_tokens
                FROM submissions s
                JOIN users u ON u.id = s.user_id
                WHERE u.leaderboard_hidden = false
                GROUP BY s.user_id
              ),
              ranked AS (
                SELECT
                  user_id,
                  RANK() OVER (ORDER BY total_tokens DESC) as rank
                FROM user_totals
              )
              SELECT rank FROM ranked WHERE user_id = ${user.id}
            `)
          : Promise.resolve([]),

        db
          .select({
            date: dailyBreakdown.date,
            timestampMs: dailyBreakdown.timestampMs,
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
    // Resolved only once the newest submitted date is known, so every window —
    // lifetime and period alike — ends on the data instead of on UTC "today".
    const rangeAnchor = getProfileRangeAnchor(stats?.latestDate, now);
    const periodRange = getProfilePeriodDateRange(period, rangeAnchor);
    const chartRange = periodRange ?? getRollingProfileDateRange(rangeAnchor);
    // Ranks over rankable users only. A hidden user is absent from the CTE
    // entirely, so this returns no row and the profile reports rank null. The
    // finite query uses the exact same anchored window as the visible profile
    // totals and chart.
    //
    // The two windows deliberately differ, because the two leaderboard tabs
    // they have to agree with do: the finite query mirrors the leaderboard's
    // period path (sequential ROW_NUMBER with the same tie-breakers, so tied
    // users get distinct positions in the same order), and the lifetime query
    // mirrors the all-time path (shared RANK, so tied users share a position).
    //
    // The scan ranks every rankable user's daily rows, so it is cached for a
    // minute per user and window instead of running on every request. A user
    // with no daily rows in the window has no row in the CTE either, so the
    // scan is skipped outright and the rank reported null directly.
    const hasPeriodRows =
      periodRange !== null &&
      dailyData.some(
        (day) => day.date >= periodRange.start && day.date <= periodRange.end,
      );
    const scopedRankResult = periodRange
      ? hasPeriodRows
        ? await unstable_cache(
            () =>
              db.execute<{ rank: number }>(sql`
                WITH user_totals AS (
                  SELECT
                    s.user_id,
                    u.username,
                    SUM(d.tokens) as total_tokens,
                    SUM(CAST(d.cost AS DECIMAL(18,4))) as total_cost
                  FROM daily_breakdown d
                  INNER JOIN submissions s ON d.submission_id = s.id
                  INNER JOIN users u ON u.id = s.user_id
                  WHERE u.leaderboard_hidden = false
                    AND d.date >= ${periodRange.start}
                    AND d.date <= ${periodRange.end}
                  GROUP BY s.user_id, u.username
                ),
                ranked AS (
                  SELECT
                    user_id,
                    ROW_NUMBER() OVER (ORDER BY total_tokens DESC, total_cost DESC, LOWER(username) ASC, user_id ASC) as rank
                  FROM user_totals
                )
                SELECT rank FROM ranked WHERE user_id = ${user.id}
              `),
            [
              "profile-period-rank",
              user.id,
              periodRange.start,
              periodRange.end,
            ],
            {
              revalidate: 60,
              tags: [
                "leaderboard",
                `user:${normalizeUsernameCacheKey(user.username)}`,
              ],
            },
          )()
        : []
      : rankResult;
    const rank =
      Number(
        (scopedRankResult as unknown as { rank: number }[])[0]?.rank,
      ) || null;

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

    /**
     * Every merge below accumulates in place, so whatever an accumulator holds
     * it will eventually rewrite. `breakdown.models` belongs to a `dailyData`
     * row, so adopting it by reference makes the accumulator and the row the
     * same object and the next row's merge silently rewrites the row that
     * seeded it. Copy the map and its entries at the boundary so the
     * accumulator only ever mutates memory it owns.
     */
    const cloneClientModels = (
      models: Record<string, ModelData> | undefined,
    ) => {
      if (!models) return undefined;

      const copy: Record<string, ModelData> = {};
      for (const [modelId, model] of Object.entries(models)) {
        copy[modelId] = { ...model };
      }
      return copy;
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
                models: cloneClientModels(breakdown.models),
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
                models: cloneClientModels(breakdown.models),
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
          tokens: Number(day.tokens),
          cost: Number(day.cost),
          inputTokens: Number(day.inputTokens),
          outputTokens: Number(day.outputTokens),
          clients,
          models,
        });
      }
    }

    // Calculate max tokens for intensity. Tokens, not cost, because every
    // embed already shades from tokens -- layoutContributions in
    // lib/embed/embedShared.ts, getUserEmbedStats and renderIsometric3DSvg all
    // recompute intensity from totalTokens -- so a cost-scaled profile graph
    // shaded the same account differently from its own embeds, and a day whose
    // client reports no pricing read as blank.
    const contributions = Array.from(aggregatedDaily.values());
    const scopedContributions = contributions.filter(
      ({ date }) => date >= chartRange.start && date <= chartRange.end,
    );
    // A lifetime request ships every day it holds — the graph's year dropdown
    // reads them. A period request ships only its window: the query fetches a
    // day or two more than the anchored window draws, and neither the intensity
    // scale nor the graph may see past the window's edge.
    const visibleContributions = periodRange
      ? scopedContributions
      : contributions;
    const maxTokens = Math.max(
      ...visibleContributions.map((c) => c.tokens),
      0,
    );
    const periodTotals = scopedContributions.reduce(
      (totals, day) => {
        totals.totalTokens += day.tokens;
        totals.totalCost += day.cost;
        totals.inputTokens += day.inputTokens;
        totals.outputTokens += day.outputTokens;

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
      },
    );

    // Build contribution graph data
    const graphContributions = visibleContributions.map((day) => {
      const intensity = calculateIntensity(day.tokens, maxTokens);

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
        rank,
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
      // Sticky per-user flag: true once any accepted submission carried a
      // backfill provenance tag (badge-only; ranking is unaffected).
      hasBackfill: latestSubmission?.hasBackfill ?? false,
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
