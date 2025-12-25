import { NextResponse } from "next/server";
import { revalidateTag } from "next/cache";
import { db, apiTokens, users, submissions, dailyBreakdown } from "@/lib/db";
import { eq, sql } from "drizzle-orm";
import {
  validateSubmission,
  generateSubmissionHash,
  type SubmissionData,
} from "@/lib/validation/submission";
import {
  mergeSourceBreakdowns,
  recalculateDayTotals,
  buildModelBreakdown,
  sourceContributionToBreakdownData,
  type SourceBreakdownData,
} from "@/lib/db/helpers";

function normalizeSubmissionData(data: unknown): void {
  if (!data || typeof data !== "object") return;
  const obj = data as Record<string, unknown>;
  if (!Array.isArray(obj.contributions)) return;

  for (const contribution of obj.contributions) {
    if (!contribution || typeof contribution !== "object") continue;
    const day = contribution as Record<string, unknown>;
    if (!Array.isArray(day.sources)) continue;

    for (const source of day.sources) {
      if (!source || typeof source !== "object") continue;
      const s = source as Record<string, unknown>;

      if (s.modelId == null || typeof s.modelId !== "string") {
        s.modelId = "unknown";
      } else {
        const trimmed = s.modelId.trim();
        s.modelId = trimmed === "" ? "unknown" : trimmed;
      }
    }
  }
}

/**
 * POST /api/submit
 * Submit token usage data from CLI
 * 
 * IMPLEMENTS SOURCE-LEVEL MERGE:
 * - Only updates sources present in submission
 * - Preserves data for sources NOT in submission
 * - Recalculates totals from dailyBreakdown
 *
 * Headers:
 *   Authorization: Bearer <api_token>
 *
 * Body: TokenContributionData JSON
 */
export async function POST(request: Request) {
  try {
    // ========================================
    // STEP 1: Authentication
    // ========================================
    const authHeader = request.headers.get("Authorization");
    if (!authHeader?.startsWith("Bearer ")) {
      return NextResponse.json(
        { error: "Missing or invalid Authorization header" },
        { status: 401 }
      );
    }

    const token = authHeader.slice(7);

    const [tokenRecord] = await db
      .select({
        tokenId: apiTokens.id,
        userId: apiTokens.userId,
        username: users.username,
        expiresAt: apiTokens.expiresAt,
      })
      .from(apiTokens)
      .innerJoin(users, eq(apiTokens.userId, users.id))
      .where(eq(apiTokens.token, token))
      .limit(1);

    if (!tokenRecord) {
      return NextResponse.json({ error: "Invalid API token" }, { status: 401 });
    }

    if (tokenRecord.expiresAt && tokenRecord.expiresAt < new Date()) {
      return NextResponse.json({ error: "API token has expired" }, { status: 401 });
    }

    // ========================================
    // STEP 2: Parse and Validate
    // ========================================
    let rawData: unknown;
    try {
      rawData = await request.json();
    } catch {
      return NextResponse.json({ error: "Invalid JSON body" }, { status: 400 });
    }

    normalizeSubmissionData(rawData);

    const validation = validateSubmission(rawData);
    if (!validation.valid || !validation.data) {
      return NextResponse.json(
        { error: "Validation failed", details: validation.errors },
        { status: 400 }
      );
    }

    // Use validated and parsed data
    const data = validation.data;

    // Reject empty submissions
    if (data.contributions.length === 0) {
      return NextResponse.json(
        { error: "No contribution data to submit" },
        { status: 400 }
      );
    }

    const submittedSources = new Set(data.summary.sources);

    // ========================================
    // STEP 3: DATABASE OPERATIONS IN TRANSACTION
    // ========================================
    const result = await db.transaction(async (tx) => {
      // Update token last used timestamp
      await tx
        .update(apiTokens)
        .set({ lastUsedAt: new Date() })
        .where(eq(apiTokens.id, tokenRecord.tokenId));

      // ------------------------------------------
      // STEP 3a: Get or create user's submission
      // NOTE: .for('update') locks the row to prevent race conditions
      // ------------------------------------------
      let [existingSubmission] = await tx
        .select({
          id: submissions.id,
        })
        .from(submissions)
        .where(eq(submissions.userId, tokenRecord.userId))
        .for('update')
        .limit(1);

      let submissionId: string;
      let isNewSubmission = false;

      if (existingSubmission) {
        submissionId = existingSubmission.id;
      } else {
        // First submission - create placeholder (totals will be calculated later)
        isNewSubmission = true;
        const [newSubmission] = await tx
          .insert(submissions)
          .values({
            userId: tokenRecord.userId,
            totalTokens: 0,
            totalCost: "0",
            inputTokens: 0,
            outputTokens: 0,
            cacheCreationTokens: 0,
            cacheReadTokens: 0,
            dateStart: data.meta.dateRange.start,
            dateEnd: data.meta.dateRange.end,
            sourcesUsed: [],
            modelsUsed: [],
            status: "verified",
            cliVersion: data.meta.version,
            submissionHash: generateSubmissionHash(data),
          })
          .returning({ id: submissions.id });

        submissionId = newSubmission.id;
      }

      // ------------------------------------------
      // STEP 3b: Fetch existing daily breakdown for merge
      // NOTE: .for('update') locks rows to prevent concurrent modification
      // ------------------------------------------
      const existingDays = await tx
        .select({
          id: dailyBreakdown.id,
          date: dailyBreakdown.date,
          sourceBreakdown: dailyBreakdown.sourceBreakdown,
        })
        .from(dailyBreakdown)
        .where(eq(dailyBreakdown.submissionId, submissionId))
        .for('update');

      // Build lookup map: date -> existing record
      const existingDaysMap = new Map(
        existingDays.map((d) => [d.date, d])
      );

      // ------------------------------------------
      // STEP 3c: Process each incoming day
      // ------------------------------------------
      for (const incomingDay of data.contributions) {
        const incomingSourceBreakdown: Record<string, SourceBreakdownData> = {};
        for (const source of incomingDay.sources) {
          const modelData = sourceContributionToBreakdownData(source);
          const existing = incomingSourceBreakdown[source.source];
          if (existing) {
            existing.tokens += modelData.tokens;
            existing.cost += modelData.cost;
            existing.input += modelData.input;
            existing.output += modelData.output;
            existing.cacheRead += modelData.cacheRead;
            existing.cacheWrite += modelData.cacheWrite;
            existing.reasoning = (existing.reasoning || 0) + modelData.reasoning;
            existing.messages += modelData.messages;
            const existingModel = existing.models[source.modelId];
            if (existingModel) {
              existingModel.tokens += modelData.tokens;
              existingModel.cost += modelData.cost;
              existingModel.input += modelData.input;
              existingModel.output += modelData.output;
              existingModel.cacheRead += modelData.cacheRead;
              existingModel.cacheWrite += modelData.cacheWrite;
              existingModel.reasoning = (existingModel.reasoning || 0) + modelData.reasoning;
              existingModel.messages += modelData.messages;
            } else {
              existing.models[source.modelId] = modelData;
            }
          } else {
            incomingSourceBreakdown[source.source] = {
              ...modelData,
              models: { [source.modelId]: modelData },
            };
          }
        }

        const existingDay = existingDaysMap.get(incomingDay.date);

        if (existingDay) {
          // ---- MERGE: Day exists, merge sources ----
          const existingSourceBreakdown = (existingDay.sourceBreakdown || {}) as Record<string, SourceBreakdownData>;

          const mergedSourceBreakdown = mergeSourceBreakdowns(
            existingSourceBreakdown,
            incomingSourceBreakdown,
            submittedSources
          );

          // Recalculate day totals from merged data
          const dayTotals = recalculateDayTotals(mergedSourceBreakdown);

          // Build modelBreakdown from merged sources
          const modelBreakdown = buildModelBreakdown(mergedSourceBreakdown);

          // UPDATE existing daily breakdown
          await tx
            .update(dailyBreakdown)
            .set({
              tokens: dayTotals.tokens,
              cost: dayTotals.cost.toFixed(4),
              inputTokens: dayTotals.inputTokens,
              outputTokens: dayTotals.outputTokens,
              sourceBreakdown: mergedSourceBreakdown,
              modelBreakdown: modelBreakdown,
            })
            .where(eq(dailyBreakdown.id, existingDay.id));
        } else {
          // ---- INSERT: New day ----
          const dayTotals = recalculateDayTotals(incomingSourceBreakdown);
          const modelBreakdown = buildModelBreakdown(incomingSourceBreakdown);

          await tx.insert(dailyBreakdown).values({
            submissionId: submissionId,
            date: incomingDay.date,
            tokens: dayTotals.tokens,
            cost: dayTotals.cost.toFixed(4),
            inputTokens: dayTotals.inputTokens,
            outputTokens: dayTotals.outputTokens,
            sourceBreakdown: incomingSourceBreakdown,
            modelBreakdown: modelBreakdown,
          });
        }
      }

      // ------------------------------------------
      // STEP 3d: Recalculate submission totals from ALL daily breakdown
      // ------------------------------------------
      const [aggregates] = await tx
        .select({
          totalTokens: sql<number>`COALESCE(SUM(${dailyBreakdown.tokens}), 0)::bigint`,
          totalCost: sql<string>`COALESCE(SUM(CAST(${dailyBreakdown.cost} AS DECIMAL(12,4))), 0)::text`,
          inputTokens: sql<number>`COALESCE(SUM(${dailyBreakdown.inputTokens}), 0)::bigint`,
          outputTokens: sql<number>`COALESCE(SUM(${dailyBreakdown.outputTokens}), 0)::bigint`,
          dateStart: sql<string>`MIN(${dailyBreakdown.date})`,
          dateEnd: sql<string>`MAX(${dailyBreakdown.date})`,
          activeDays: sql<number>`COUNT(CASE WHEN ${dailyBreakdown.tokens} > 0 THEN 1 END)::int`,
          rowCount: sql<number>`COUNT(*)::int`,
        })
        .from(dailyBreakdown)
        .where(eq(dailyBreakdown.submissionId, submissionId));

      // Collect all unique sources and models from ALL daily breakdown records
      const allDays = await tx
        .select({
          sourceBreakdown: dailyBreakdown.sourceBreakdown,
        })
        .from(dailyBreakdown)
        .where(eq(dailyBreakdown.submissionId, submissionId));

      const allSources = new Set<string>();
      const allModels = new Set<string>();
      let totalCacheRead = 0;
      let totalCacheCreation = 0;
      let totalReasoning = 0;

      for (const day of allDays) {
        if (day.sourceBreakdown) {
          for (const [sourceName, sourceData] of Object.entries(day.sourceBreakdown)) {
            allSources.add(sourceName);
            const sd = sourceData as SourceBreakdownData;
            if (sd.models) {
              for (const modelId of Object.keys(sd.models)) {
                allModels.add(modelId);
              }
            } else if (sd.modelId) {
              allModels.add(sd.modelId);
            }
            totalCacheRead += sd.cacheRead || 0;
            totalCacheCreation += sd.cacheWrite || 0;
            totalReasoning += sd.reasoning || 0;
          }
        }
      }

      // ------------------------------------------
      // STEP 3e: Update submission record
      // ------------------------------------------
      await tx
        .update(submissions)
        .set({
          totalTokens: aggregates.totalTokens,
          totalCost: aggregates.totalCost,
          inputTokens: aggregates.inputTokens,
          outputTokens: aggregates.outputTokens,
          cacheReadTokens: totalCacheRead,
          cacheCreationTokens: totalCacheCreation,
          reasoningTokens: totalReasoning,
          dateStart: aggregates.dateStart,
          dateEnd: aggregates.dateEnd,
          sourcesUsed: Array.from(allSources),
          modelsUsed: Array.from(allModels),
          cliVersion: data.meta.version,
          submissionHash: generateSubmissionHash(data),
          submitCount: sql`COALESCE(submit_count, 0) + 1`,
          updatedAt: new Date(),
        })
        .where(eq(submissions.id, submissionId));

      return {
        submissionId,
        isNewSubmission,
        metrics: {
          totalTokens: aggregates.totalTokens,
          totalCost: parseFloat(aggregates.totalCost),
          dateRange: {
            start: aggregates.dateStart,
            end: aggregates.dateEnd,
          },
          activeDays: aggregates.activeDays,
          sources: Array.from(allSources),
        },
      };
    });

    try {
      revalidateTag("leaderboard", "max");
      revalidateTag(`user:${tokenRecord.username}`, "max");
    } catch (e) {
      console.error("Cache invalidation failed:", e);
    }

    return NextResponse.json({
      success: true,
      submissionId: result.submissionId,
      username: tokenRecord.username,
      metrics: result.metrics,
      mode: result.isNewSubmission ? "create" : "merge",
      warnings: validation.warnings.length > 0 ? validation.warnings : undefined,
    });
  } catch (error) {
    console.error("Submit error:", error);
    return NextResponse.json(
      { error: "Internal server error" },
      { status: 500 }
    );
  }
}
