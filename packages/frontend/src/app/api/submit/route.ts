import { NextResponse } from "next/server";
import { revalidateTag } from "next/cache";
import { db, apiTokens, submissions, dailyBreakdown } from "@/lib/db";
import { eq, sql } from "drizzle-orm";
import {
  validateSubmission,
  generateSubmissionHash,
  type SubmissionData,
} from "@/lib/validation/submission";
import { authenticatePersonalToken } from "@/lib/auth/personalTokens";
import {
  mergeClientBreakdowns,
  recalculateDayTotals,
  buildModelBreakdown,
  clientContributionToBreakdownData,
  mergeTimestampMs,
  type ClientBreakdownData,
} from "@/lib/db/helpers";

function normalizeSubmissionData(data: unknown): void {
  if (!data || typeof data !== "object") return;
  const obj = data as Record<string, unknown>;
  if (!Array.isArray(obj.contributions)) return;

  for (const contribution of obj.contributions) {
    if (!contribution || typeof contribution !== "object") continue;
    const day = contribution as Record<string, unknown>;
    // Handle both legacy "sources" and new "clients" formats
    const items = Array.isArray(day.sources)
      ? day.sources
      : Array.isArray(day.clients)
      ? day.clients
      : null;
    if (!items) continue;
    for (const entry of items) {
      if (!entry || typeof entry !== "object") continue;
      const s = entry as Record<string, unknown>;
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
 * IMPLEMENTS CLIENT-LEVEL MERGE:
 * - Only updates clients present in submission
 * - Preserves data for clients NOT in submission
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
    const authResult = await authenticatePersonalToken(token, {
      touchLastUsedAt: false,
    });

    if (authResult.status === "invalid") {
      return NextResponse.json({ error: "Invalid API token" }, { status: 401 });
    }

    if (authResult.status === "expired") {
      return NextResponse.json({ error: "API token has expired" }, { status: 401 });
    }

    const tokenRecord = authResult;

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

    const data = validation.data;

    if (data.contributions.length === 0) {
      return NextResponse.json(
        { error: "No contribution data to submit" },
        { status: 400 }
      );
    }

    const submittedClients = new Set<SubmissionData["summary"]["clients"][number]>(data.summary.clients);
    for (const contribution of data.contributions) {
      for (const client_contrib of contribution.clients) {
        submittedClients.add(client_contrib.client);
      }
    }
    if (submittedClients.has("kilo")) {
      submittedClients.add("kilocode" as SubmissionData["summary"]["clients"][number]);
    }
    const hashData: SubmissionData = {
      ...data,
      summary: {
        ...data.summary,
        clients: Array.from(submittedClients).sort(),
      },
    };

    // ========================================
    // STEP 3: DATABASE OPERATIONS IN TRANSACTION
    // ========================================
    const result = await db.transaction(async (tx) => {
      await tx
        .update(apiTokens)
        .set({ lastUsedAt: new Date() })
        .where(eq(apiTokens.id, tokenRecord.tokenId));

      // ------------------------------------------
      // STEP 3a: Get or create user's submission
      // ------------------------------------------
      const [existingSubmission] = await tx
        .select({ id: submissions.id })
        .from(submissions)
        .where(eq(submissions.userId, tokenRecord.userId))
        .for('update')
        .limit(1);

      let submissionId: string;
      let isNewSubmission = false;

      if (existingSubmission) {
        submissionId = existingSubmission.id;
      } else {
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
            submissionHash: generateSubmissionHash(hashData),
          })
          .returning({ id: submissions.id });

        submissionId = newSubmission.id;
      }

      // ------------------------------------------
      // STEP 3b: Fetch existing daily breakdown for merge
      // ------------------------------------------
      const existingDays = await tx
        .select({
          id: dailyBreakdown.id,
          date: dailyBreakdown.date,
          timestampMs: dailyBreakdown.timestampMs,
          sourceBreakdown: dailyBreakdown.sourceBreakdown,
        })
        .from(dailyBreakdown)
        .where(eq(dailyBreakdown.submissionId, submissionId))
        .for('update');

      const existingDaysMap = new Map(
        existingDays.map((d) => [d.date, d])
      );

      // ------------------------------------------
      // STEP 3c: Compute merge results in memory, then batch write
      // ------------------------------------------
      const toInsert: Array<{
        submissionId: string;
        date: string;
        tokens: number;
        cost: string;
        inputTokens: number;
        outputTokens: number;
        timestampMs: number | null;
        sourceBreakdown: Record<string, ClientBreakdownData>;
        modelBreakdown: Record<string, number>;
      }> = [];

      const toUpdate: Array<{
        id: string;
        tokens: number;
        cost: string;
        inputTokens: number;
        outputTokens: number;
        timestampMs: number | null;
        sourceBreakdown: Record<string, ClientBreakdownData>;
        modelBreakdown: Record<string, number>;
      }> = [];

      for (const incomingDay of data.contributions) {
        const incomingClientBreakdown: Record<string, ClientBreakdownData> = {};
        for (const client_contrib of incomingDay.clients) {
          const modelData = clientContributionToBreakdownData(client_contrib);
          const existing = incomingClientBreakdown[client_contrib.client];
          if (existing) {
            existing.tokens += modelData.tokens;
            existing.cost += modelData.cost;
            existing.input += modelData.input;
            existing.output += modelData.output;
            existing.cacheRead += modelData.cacheRead;
            existing.cacheWrite += modelData.cacheWrite;
            existing.reasoning = (existing.reasoning || 0) + modelData.reasoning;
            existing.messages += modelData.messages;
            const existingModel = existing.models[client_contrib.modelId];
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
              existing.models[client_contrib.modelId] = modelData;
            }
          } else {
            incomingClientBreakdown[client_contrib.client] = {
              ...modelData,
              models: { [client_contrib.modelId]: modelData },
            };
          }
        }

        const existingDay = existingDaysMap.get(incomingDay.date);

        if (existingDay) {
           const existingClientBreakdown = (existingDay.sourceBreakdown || {}) as Record<string, ClientBreakdownData>;
           const mergedClientBreakdown = mergeClientBreakdowns(
             existingClientBreakdown,
             incomingClientBreakdown,
             submittedClients
           );
          const dayTotals = recalculateDayTotals(mergedClientBreakdown);
          const modelBreakdown = buildModelBreakdown(mergedClientBreakdown);

          toUpdate.push({
            id: existingDay.id,
            tokens: dayTotals.tokens,
            cost: dayTotals.cost.toFixed(4),
            inputTokens: dayTotals.inputTokens,
            outputTokens: dayTotals.outputTokens,
            timestampMs: mergeTimestampMs(existingDay.timestampMs, incomingDay.timestampMs ?? null),
            sourceBreakdown: mergedClientBreakdown,
            modelBreakdown,
          });
        } else {
          const dayTotals = recalculateDayTotals(incomingClientBreakdown);
          const modelBreakdown = buildModelBreakdown(incomingClientBreakdown);

          toInsert.push({
            submissionId,
            date: incomingDay.date,
            tokens: dayTotals.tokens,
            cost: dayTotals.cost.toFixed(4),
            inputTokens: dayTotals.inputTokens,
            outputTokens: dayTotals.outputTokens,
            timestampMs: incomingDay.timestampMs ?? null,
            sourceBreakdown: incomingClientBreakdown,
            modelBreakdown,
          });
        }
      }

      // Batch INSERT new days
      if (toInsert.length > 0) {
        await tx.insert(dailyBreakdown).values(toInsert);
      }

      // Batch UPDATE existing days via raw SQL VALUES list
      if (toUpdate.length > 0) {
        const valuesClauses = toUpdate.map(
          (row) =>
            sql`(${row.id}::uuid, ${row.tokens}::bigint, ${row.cost}::numeric(10,4), ${row.inputTokens}::bigint, ${row.outputTokens}::bigint, ${row.timestampMs}::bigint, ${JSON.stringify(row.sourceBreakdown)}::jsonb, ${JSON.stringify(row.modelBreakdown)}::jsonb)`
        );

        const valuesList = sql.join(valuesClauses, sql`, `);

        await tx.execute(sql`
          UPDATE daily_breakdown AS d SET
            tokens = batch.tokens,
            cost = batch.cost,
            input_tokens = batch.input_tokens,
            output_tokens = batch.output_tokens,
            timestamp_ms = batch.timestamp_ms,
            source_breakdown = batch.source_breakdown,
            model_breakdown = batch.model_breakdown
          FROM (VALUES ${valuesList})
            AS batch(id, tokens, cost, input_tokens, output_tokens, timestamp_ms, source_breakdown, model_breakdown)
          WHERE d.id = batch.id
        `);
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

      const allDays = await tx
        .select({
          sourceBreakdown: dailyBreakdown.sourceBreakdown,
        })
        .from(dailyBreakdown)
        .where(eq(dailyBreakdown.submissionId, submissionId));

      const allClients = new Set<string>();
      const allModels = new Set<string>();
      let totalCacheRead = 0;
      let totalCacheCreation = 0;
      let totalReasoning = 0;

      for (const day of allDays) {
        if (day.sourceBreakdown) {
          for (const [rawClientName, clientData] of Object.entries(day.sourceBreakdown)) {
            const clientName = rawClientName === "kilocode" ? "kilo" : rawClientName;
            allClients.add(clientName);
            const cd = clientData as ClientBreakdownData;
            if (cd.models) {
              for (const modelId of Object.keys(cd.models)) {
                allModels.add(modelId);
              }
            } else if (cd.modelId) {
              allModels.add(cd.modelId);
            }
            totalCacheRead += cd.cacheRead || 0;
            totalCacheCreation += cd.cacheWrite || 0;
            totalReasoning += cd.reasoning || 0;
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
           sourcesUsed: Array.from(allClients),
           modelsUsed: Array.from(allModels),
          cliVersion: data.meta.version,
          submissionHash: generateSubmissionHash(hashData),
          submitCount: sql`COALESCE(submit_count, 0) + 1`,
          schemaVersion: sql`GREATEST(COALESCE(${submissions.schemaVersion}, 0), ${data.contributions.some((c) => c.timestampMs != null) ? 1 : 0})`,
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
          clients: Array.from(allClients),
        },
      };
    });

    try {
      revalidateTag("leaderboard", "max");
      revalidateTag(`user:${tokenRecord.username}`, "max");
      revalidateTag("user-rank", "max");
      revalidateTag(`user-rank:${tokenRecord.username}`, "max");
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
