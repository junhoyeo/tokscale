import { NextResponse } from "next/server";
import { revalidateTag } from "next/cache";
import { db, apiTokens, submissions, submittedDevices, dailyBreakdown } from "@/lib/db";
import { and, eq, sql } from "drizzle-orm";
import {
  validateSubmission,
  generateSubmissionHash,
  type SubmissionData,
} from "@/lib/validation/submission";
import { authenticatePersonalToken } from "@/lib/auth/personalTokens";
import { getBearerToken } from "../../../lib/auth/bearerToken";
import {
  mergeClientBreakdownsWithRegressionGuard,
  recalculateDayTotals,
  clientContributionToBreakdownData,
  deriveClientBreakdownProvenance,
  mergeTimestampMs,
  type ClientBreakdownData,
} from "@/lib/db/helpers";
import { normalizeUsernameCacheKey, revalidateUsernamePaths } from "@/lib/db/usernameLookup";
import { revalidateUserGroupLeaderboards } from "@/lib/groups/cache";
import { LEGACY_DEVICE_KEY } from "@/lib/devices/shared";

const LEGACY_SUBMIT_DEVICE_KEY = LEGACY_DEVICE_KEY;
const LEGACY_SUBMIT_DEVICE_NAME = "Legacy submissions";
// PostgreSQL caps a single statement at 65,535 bound parameters. Each
// inserted row binds 10 params, so chunk large backfills (e.g. ~6,500+ days)
// across multiple INSERT statements to stay well under that limit.
const INSERT_CHUNK_SIZE = 1000;

// "kilocode" is a legacy alias of "kilo" (mirrors LEGACY_CLIENT_ALIASES in
// lib/validation/submission.ts and lib/publicProfileData.ts). Incoming
// contributions are already normalized to "kilo" by validateSubmission's
// preprocessing, but a daily_breakdown row's stored source_breakdown JSON
// can still carry a "kilocode" key written before that normalization
// existed. Fold it into "kilo" before merging so the day-level merge treats
// the two as the SAME client instead of summing them as disjoint clients
// (which would double-count the same underlying usage).
const LEGACY_CLIENT_ALIASES: Record<string, string> = { kilocode: "kilo" };

function mergeModelBreakdowns(
  target: Record<string, ClientBreakdownData["models"][string]>,
  incoming: Record<string, ClientBreakdownData["models"][string]>
): void {
  for (const [modelId, modelData] of Object.entries(incoming)) {
    const existingModel = target[modelId];
    if (existingModel) {
      existingModel.tokens += modelData.tokens || 0;
      existingModel.cost += modelData.cost || 0;
      existingModel.input += modelData.input || 0;
      existingModel.output += modelData.output || 0;
      existingModel.cacheRead += modelData.cacheRead || 0;
      existingModel.cacheWrite += modelData.cacheWrite || 0;
      existingModel.reasoning = (existingModel.reasoning || 0) + (modelData.reasoning || 0);
      existingModel.messages += modelData.messages || 0;
    } else {
      target[modelId] = { ...modelData };
    }
  }
}

function normalizeClientBreakdownAliases(
  breakdown: Record<string, ClientBreakdownData>
): Record<string, ClientBreakdownData> {
  const normalized: Record<string, ClientBreakdownData> = {};

  for (const [rawClientName, data] of Object.entries(breakdown)) {
    const clientName = LEGACY_CLIENT_ALIASES[rawClientName] ?? rawClientName;
    const existing = normalized[clientName];

    if (!existing) {
      normalized[clientName] = { ...data, models: { ...data.models } };
      continue;
    }

    existing.tokens += data.tokens || 0;
    existing.cost += data.cost || 0;
    existing.input += data.input || 0;
    existing.output += data.output || 0;
    existing.cacheRead += data.cacheRead || 0;
    existing.cacheWrite += data.cacheWrite || 0;
    existing.reasoning = (existing.reasoning || 0) + (data.reasoning || 0);
    existing.messages += data.messages || 0;
    mergeModelBreakdowns(existing.models, data.models || {});
    existing.provenance = deriveClientBreakdownProvenance(existing);
  }

  return normalized;
}

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

// Submission schema versions:
//   0 = legacy CLI: no per-day timestamps, no device metadata.
//   1 = timestamp-aware CLI (>=v2.1): per-day `timestampMs` set, still no device.
//   2 = device-aware CLI (>=v2.1.x post-#517): caller sends a `device` object,
//       so daily_breakdown rows are keyed by submittedDeviceId.
// The submissions row keeps the GREATEST() of stored vs. incoming so a single
// device-aware submit cannot regress an account back to v1 hash semantics.
function getSubmitDevice(data: SubmissionData): { key: string; name: string | null; schemaVersion: number } {
  if (data.device) {
    return {
      key: data.device.id,
      name: data.device.name ?? null,
      schemaVersion: 2,
    };
  }

  return {
    key: LEGACY_SUBMIT_DEVICE_KEY,
    name: LEGACY_SUBMIT_DEVICE_NAME,
    schemaVersion: data.contributions.some((c) => c.timestampMs != null) ? 1 : 0,
  };
}

function isUniqueConstraintViolation(error: unknown): boolean {
  if (!error || typeof error !== "object") return false;
  const maybeError = error as { code?: unknown; cause?: unknown };
  if (maybeError.code === "23505") return true;
  const cause = maybeError.cause;
  return Boolean(cause && typeof cause === "object" && (cause as { code?: unknown }).code === "23505");
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
    const token = getBearerToken(request.headers.get("Authorization"));
    if (!token) {
      return NextResponse.json(
        { error: "Missing or invalid Authorization header" },
        { status: 401 }
      );
    }

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

    const mcpServers: string[] | null =
      rawData != null && typeof rawData === "object" &&
      Array.isArray((rawData as Record<string, unknown>).mcpServers)
        ? ((rawData as Record<string, unknown>).mcpServers as unknown[]).filter(
            (s): s is string => typeof s === "string" && s.length > 0
          )
        : null;

    const validation = validateSubmission(rawData);

    if (!validation.valid || !validation.data) {
      return NextResponse.json(
        { error: "Validation failed", details: validation.errors },
        { status: 400 }
      );
    }

    const data = validation.data;
    const warnings = [...validation.warnings];

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
        try {
          const [newSubmission] = await tx.transaction(async (sp) =>
            sp
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
                cliVersion: data.meta.version,
                submissionHash: generateSubmissionHash(hashData),
              })
              .returning({ id: submissions.id })
          );

          submissionId = newSubmission.id;
          isNewSubmission = true;
        } catch (creationErr) {
          if (!isUniqueConstraintViolation(creationErr)) {
            throw creationErr;
          }

          const [racedSubmission] = await tx
            .select({ id: submissions.id })
            .from(submissions)
            .where(eq(submissions.userId, tokenRecord.userId))
            .for('update')
            .limit(1);

          if (!racedSubmission) {
            throw creationErr;
          }

          submissionId = racedSubmission.id;
        }
      }

      const submitDevice = getSubmitDevice(data);
      const submittedAt = new Date();
      const [submittedDevice] = await tx
        .insert(submittedDevices)
        .values({
          userId: tokenRecord.userId,
          deviceKey: submitDevice.key,
          displayName: submitDevice.name,
          lastSubmittedAt: submittedAt,
          updatedAt: submittedAt,
        })
        .onConflictDoUpdate({
          target: [submittedDevices.userId, submittedDevices.deviceKey],
          set: {
            displayName: sql`COALESCE(EXCLUDED.display_name, ${submittedDevices.displayName})`,
            lastSubmittedAt: submittedAt,
            updatedAt: submittedAt,
          },
        })
        .returning({ id: submittedDevices.id });

      // ------------------------------------------
      // STEP 3b: Fetch existing daily breakdown for merge
      // ------------------------------------------
      const fetchExistingDeviceDays = () =>
        tx
          .select({
            id: dailyBreakdown.id,
            date: dailyBreakdown.date,
            timestampMs: dailyBreakdown.timestampMs,
            activeTimeMs: dailyBreakdown.activeTimeMs,
            sourceBreakdown: dailyBreakdown.sourceBreakdown,
          })
          .from(dailyBreakdown)
          .where(
            and(
              eq(dailyBreakdown.submissionId, submissionId),
              eq(dailyBreakdown.submittedDeviceId, submittedDevice.id)
            )
          )
          .for('update');

      let existingDeviceDays = await fetchExistingDeviceDays();

      if (
        existingDeviceDays.length === 0 &&
        !isNewSubmission &&
        submitDevice.key !== LEGACY_SUBMIT_DEVICE_KEY
      ) {
        // The first device-aware submit after the migration should continue
        // the user's legacy bucket instead of counting the same history twice.
        // Once any modern device rows exist, attribution is ambiguous, so the
        // legacy bucket stays separate.
        //
        // Race note: two concurrent submits from the same user can both reach
        // this branch before either has committed. The second UPDATE will try
        // to re-stamp submitted_device_id on rows the first already claimed,
        // which can violate the (submission_id, submitted_device_id, date)
        // unique constraint. The ON CONFLICT DO NOTHING below makes the UPDATE
        // skip conflicting rows rather than throw, and the outer try/catch falls
        // through to the normal insert path if a unique violation still escapes
        // (e.g. via a concurrent INSERT racing the UPDATE window).
        try {
          // Wrap the UPDATE in a savepoint so a unique-constraint violation
          // from a concurrent submit does not poison the enclosing
          // transaction. Drizzle's nested transaction maps to a Postgres
          // SAVEPOINT; throwing inside the inner block rolls back to the
          // savepoint and leaves the outer tx in a usable state.
          await tx.transaction(async (sp) => {
            await sp.execute(sql`
              UPDATE daily_breakdown AS db
              SET submitted_device_id = ${submittedDevice.id}
              WHERE db.submission_id = ${submissionId}
                AND db.submitted_device_id IN (
                  SELECT sd.id
                  FROM submitted_devices AS sd
                  WHERE sd.user_id = ${tokenRecord.userId}
                    AND sd.device_key = ${LEGACY_SUBMIT_DEVICE_KEY}
                )
                AND NOT EXISTS (
                  SELECT 1
                  FROM daily_breakdown AS modern
                  WHERE modern.submission_id = db.submission_id
                    AND modern.submitted_device_id NOT IN (
                      SELECT sd2.id
                      FROM submitted_devices AS sd2
                      WHERE sd2.user_id = ${tokenRecord.userId}
                        AND sd2.device_key = ${LEGACY_SUBMIT_DEVICE_KEY}
                    )
                )
                AND NOT EXISTS (
                  SELECT 1
                  FROM daily_breakdown AS dup
                  WHERE dup.submission_id = db.submission_id
                    AND dup.submitted_device_id = ${submittedDevice.id}
                    AND dup.date = db.date
                )
            `);
          });
        } catch (adoptionErr) {
          // Unique constraint hit from a concurrent submit racing this UPDATE.
          // Savepoint rolled back; outer tx is still usable.
          // fetchExistingDeviceDays() below will pick up rows already claimed
          // by the other request, and subsequent logic will merge rather than
          // re-adopt.
          console.warn("Legacy adoption conflict (concurrent submit), falling through:", adoptionErr);
        }
        existingDeviceDays = await fetchExistingDeviceDays();
      }

      // Fetch existing days across the WHOLE submission (every device), not
      // just the submitting device. A date already recorded by another
      // device must flow through the toUpdate merge path below (which runs
      // mergeClientBreakdownsWithRegressionGuard) instead of falling into
      // toInsert, where ON CONFLICT DO UPDATE would blindly overwrite the
      // other device's row with this device's raw values.
      const existingDays = await tx
        .select({
          id: dailyBreakdown.id,
          date: dailyBreakdown.date,
          timestampMs: dailyBreakdown.timestampMs,
          activeTimeMs: dailyBreakdown.activeTimeMs,
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
        submittedDeviceId: string;
        date: string;
        tokens: number;
        cost: string;
        inputTokens: number;
        outputTokens: number;
        timestampMs: number | null;
        activeTimeMs: number | null;
        sourceBreakdown: Record<string, ClientBreakdownData>;
      }> = [];

      const toUpdate: Array<{
        id: string;
        submittedDeviceId: string;
        tokens: number;
        cost: string;
        inputTokens: number;
        outputTokens: number;
        timestampMs: number | null;
        activeTimeMs: number | null;
        sourceBreakdown: Record<string, ClientBreakdownData>;
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
            existing.provenance = deriveClientBreakdownProvenance(existing);
          } else {
            const clientBreakdown = {
              ...modelData,
              models: { [client_contrib.modelId]: modelData },
            };
            incomingClientBreakdown[client_contrib.client] = {
              ...clientBreakdown,
              provenance: deriveClientBreakdownProvenance(clientBreakdown),
            };
          }
        }

        const existingDay = existingDaysMap.get(incomingDay.date);

        if (existingDay) {
          const existingClientBreakdown = normalizeClientBreakdownAliases(
            (existingDay.sourceBreakdown || {}) as Record<string, ClientBreakdownData>
          );
          const mergeResult = mergeClientBreakdownsWithRegressionGuard(
            existingClientBreakdown,
            incomingClientBreakdown,
            submittedClients
          );
          warnings.push(
            ...mergeResult.warnings.map((warning) => `Day ${incomingDay.date}: ${warning}`)
          );
          const mergedClientBreakdown = mergeResult.merged;
          const dayTotals = recalculateDayTotals(mergedClientBreakdown);

          toUpdate.push({
            id: existingDay.id,
            submittedDeviceId: submittedDevice.id,
            tokens: dayTotals.tokens,
            cost: dayTotals.cost.toFixed(4),
            inputTokens: dayTotals.inputTokens,
            outputTokens: dayTotals.outputTokens,
            timestampMs: mergeTimestampMs(existingDay.timestampMs, incomingDay.timestampMs ?? null),
            activeTimeMs: incomingDay.activeTimeMs ?? existingDay.activeTimeMs ?? null,
            sourceBreakdown: mergedClientBreakdown,
          });
        } else {
          const dayTotals = recalculateDayTotals(incomingClientBreakdown);

          toInsert.push({
            submissionId,
            submittedDeviceId: submittedDevice.id,
            date: incomingDay.date,
            tokens: dayTotals.tokens,
            cost: dayTotals.cost.toFixed(4),
            inputTokens: dayTotals.inputTokens,
            outputTokens: dayTotals.outputTokens,
            timestampMs: incomingDay.timestampMs ?? null,
            activeTimeMs: incomingDay.activeTimeMs ?? null,
            sourceBreakdown: incomingClientBreakdown,
          });
        }
      }

      // Batch INSERT new days via raw SQL VALUES list, chunked to stay under
      // PostgreSQL's 65,535 bound-parameter limit (10 params/row here --
      // a large historical backfill can otherwise exceed it in one statement).
      // ON CONFLICT (submission_id, date) DO UPDATE is a defensive fallback
      // for a concurrent submit racing this one between the SELECT above and
      // this INSERT -- existingDaysMap already covers every device for this
      // submission, so under normal (non-racing) conditions every row here is
      // a genuinely new date.
      for (let i = 0; i < toInsert.length; i += INSERT_CHUNK_SIZE) {
        const chunk = toInsert.slice(i, i + INSERT_CHUNK_SIZE);
        const insertValuesClauses = chunk.map(
          (row) =>
            sql`(${row.submissionId}::uuid, ${row.submittedDeviceId}::uuid, ${row.date}, ${row.tokens}::bigint, ${row.cost}::numeric(14,4), ${row.inputTokens}::bigint, ${row.outputTokens}::bigint, ${row.timestampMs}::bigint, ${row.activeTimeMs}::bigint, ${JSON.stringify(row.sourceBreakdown)}::jsonb)`
        );

        const insertValuesList = sql.join(insertValuesClauses, sql`, `);

        await tx.execute(sql`
          INSERT INTO daily_breakdown (
            submission_id, submitted_device_id, date, tokens, cost,
            input_tokens, output_tokens, timestamp_ms, active_time_ms, source_breakdown
          )
          VALUES ${insertValuesList}
          ON CONFLICT (submission_id, date) DO UPDATE SET
            submitted_device_id = EXCLUDED.submitted_device_id,
            tokens = EXCLUDED.tokens,
            cost = EXCLUDED.cost,
            input_tokens = EXCLUDED.input_tokens,
            output_tokens = EXCLUDED.output_tokens,
            timestamp_ms = EXCLUDED.timestamp_ms,
            active_time_ms = EXCLUDED.active_time_ms,
            source_breakdown = EXCLUDED.source_breakdown
        `);
      }

      // Batch UPDATE existing days via raw SQL VALUES list, chunked for the
      // same parameter-limit reason as the INSERT above. Includes
      // submitted_device_id so a replacement-device submit re-attributes the
      // merged row to the submitting device instead of leaving it stamped
      // with the previous device that first created the row.
      for (let i = 0; i < toUpdate.length; i += INSERT_CHUNK_SIZE) {
        const chunk = toUpdate.slice(i, i + INSERT_CHUNK_SIZE);
        const valuesClauses = chunk.map(
          (row) =>
            sql`(${row.id}::uuid, ${row.submittedDeviceId}::uuid, ${row.tokens}::bigint, ${row.cost}::numeric(14,4), ${row.inputTokens}::bigint, ${row.outputTokens}::bigint, ${row.timestampMs}::bigint, ${row.activeTimeMs}::bigint, ${JSON.stringify(row.sourceBreakdown)}::jsonb)`
        );

        const valuesList = sql.join(valuesClauses, sql`, `);

        await tx.execute(sql`
          UPDATE daily_breakdown AS d SET
            submitted_device_id = batch.submitted_device_id,
            tokens = batch.tokens,
            cost = batch.cost,
            input_tokens = batch.input_tokens,
            output_tokens = batch.output_tokens,
            timestamp_ms = batch.timestamp_ms,
            active_time_ms = batch.active_time_ms,
            source_breakdown = batch.source_breakdown
          FROM (VALUES ${valuesList})
            AS batch(id, submitted_device_id, tokens, cost, input_tokens, output_tokens, timestamp_ms, active_time_ms, source_breakdown)
          WHERE d.id = batch.id
        `);
      }

      // ------------------------------------------
      // STEP 3d: Recalculate submission totals from ALL daily breakdown
      // ------------------------------------------
      const [aggregates] = await tx
        .select({
          totalTokens: sql<number>`COALESCE(SUM(${dailyBreakdown.tokens}), 0)::bigint`,
          totalCost: sql<string>`COALESCE(SUM(CAST(${dailyBreakdown.cost} AS DECIMAL(14,4))), 0)::text`,
          inputTokens: sql<number>`COALESCE(SUM(${dailyBreakdown.inputTokens}), 0)::bigint`,
          outputTokens: sql<number>`COALESCE(SUM(${dailyBreakdown.outputTokens}), 0)::bigint`,
          dateStart: sql<string>`MIN(${dailyBreakdown.date})`,
          dateEnd: sql<string>`MAX(${dailyBreakdown.date})`,
          activeDays: sql<number>`COUNT(DISTINCT CASE WHEN ${dailyBreakdown.tokens} > 0 THEN ${dailyBreakdown.date} END)::int`,
          totalActiveTimeMs: sql<number>`COALESCE(SUM(${dailyBreakdown.activeTimeMs}), 0)::bigint`,
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
          schemaVersion: sql`GREATEST(COALESCE(${submissions.schemaVersion}, 0), ${submitDevice.schemaVersion})`,
          totalActiveTimeMs: aggregates.totalActiveTimeMs,
          // Session-shape metrics cannot be safely recomputed from daily active-time buckets.
          ...(data.timeMetrics ? {
            longestContinuousMs: data.timeMetrics.longestContinuousMs,
            maxConcurrentSessions: data.timeMetrics.maxConcurrentSessions,
            sessionCount: data.timeMetrics.sessionCount,
          } : {}),
          mcpServers: mcpServers && mcpServers.length > 0 ? mcpServers : null,
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

    const usernameCacheKey = normalizeUsernameCacheKey(tokenRecord.username);
    try {
      revalidateTag("leaderboard", "max");
      revalidateTag(`user:${usernameCacheKey}`, "max");
      revalidateTag("user-rank", "max");
      revalidateTag(`user-rank:${usernameCacheKey}`, "max");
    } catch (e) {
      console.error("Public cache invalidation failed:", e);
    }

    try {
      await revalidateUserGroupLeaderboards(tokenRecord.userId);
    } catch (e) {
      console.error("Group leaderboard cache invalidation failed:", e);
    }

    try {
      revalidateUsernamePaths(tokenRecord.username);
    } catch (e) {
      console.error("Username path revalidation failed:", e);
    }

    return NextResponse.json({
      success: true,
      submissionId: result.submissionId,
      username: tokenRecord.username,
      metrics: result.metrics,
      mode: result.isNewSubmission ? "create" : "merge",
      warnings: warnings.length > 0 ? warnings : undefined,
    });
  } catch (error) {
    console.error("Submit error:", error);
    return NextResponse.json(
      { error: "Internal server error" },
      { status: 500 }
    );
  }
}
