import { and, eq, sql } from "drizzle-orm";
import { db, dailyBreakdown, submittedDevices, submissions } from "./db";
import {
  clientContributionToBreakdownData,
  deriveClientBreakdownProvenance,
  mergeClientBreakdownsWithRegressionGuard,
  mergeTimestampMs,
  recalculateDayTotals,
  type ClientBreakdownData,
} from "./db/helpers";
import { LEGACY_DEVICE_KEY } from "./devices/shared";
import {
  generateSubmissionHash,
  type SubmissionData,
} from "./validation/submission";

const LEGACY_SUBMIT_DEVICE_NAME = "Legacy submissions";

export type SubmissionTransaction = Parameters<
  Parameters<typeof db.transaction>[0]
>[0];

interface ExistingDeviceDay {
  id: string;
  date: string;
  timestampMs: number | null;
  activeTimeMs: number | null;
  sourceBreakdown: unknown;
}

export interface SubmitDevice {
  key: string;
  name: string | null;
  schemaVersion: number;
}

export interface SubmissionPersistenceMetrics {
  totalTokens: number;
  totalCost: number;
  dateRange: {
    start: string;
    end: string;
  };
  activeDays: number;
  clients: string[];
}

export interface TrustedSubmissionResult {
  submissionId: string;
  isNewSubmission: boolean;
  metrics: SubmissionPersistenceMetrics;
  warnings: string[];
}

export function getSubmitDevice(data: SubmissionData): SubmitDevice {
  if (data.device) {
    return {
      key: data.device.id,
      name: data.device.name ?? null,
      schemaVersion: 2,
    };
  }

  return {
    key: LEGACY_DEVICE_KEY,
    name: LEGACY_SUBMIT_DEVICE_NAME,
    schemaVersion: data.contributions.some((day) => day.timestampMs != null)
      ? 1
      : 0,
  };
}

export function collectSubmittedClients(
  data: SubmissionData
): Set<SubmissionData["summary"]["clients"][number]> {
  const submittedClients = new Set<
    SubmissionData["summary"]["clients"][number]
  >(data.summary.clients);

  for (const contribution of data.contributions) {
    for (const clientContribution of contribution.clients) {
      submittedClients.add(clientContribution.client);
    }
  }

  if (submittedClients.has("kilo")) {
    submittedClients.add(
      "kilocode" as SubmissionData["summary"]["clients"][number]
    );
  }

  return submittedClients;
}

function isUniqueConstraintViolation(error: unknown): boolean {
  if (!error || typeof error !== "object") {
    return false;
  }

  const maybeError = error as { code?: unknown; cause?: unknown };
  if (maybeError.code === "23505") {
    return true;
  }

  const cause = maybeError.cause;
  return Boolean(
    cause &&
      typeof cause === "object" &&
      (cause as { code?: unknown }).code === "23505"
  );
}

export async function applyTrustedSubmission(
  tx: SubmissionTransaction,
  {
    userId,
    data,
    mcpServers = null,
  }: {
    userId: string;
    data: SubmissionData;
    mcpServers?: string[] | null;
  }
): Promise<TrustedSubmissionResult> {
  const warnings: string[] = [];
  const submitDevice = getSubmitDevice(data);
  const submittedClients = collectSubmittedClients(data);
  const hashData: SubmissionData = {
    ...data,
    summary: {
      ...data.summary,
      clients: Array.from(submittedClients).sort(),
    },
  };

  const [existingSubmission] = await tx
    .select({ id: submissions.id })
    .from(submissions)
    .where(eq(submissions.userId, userId))
    .for("update")
    .limit(1);

  let submissionId: string;
  let isNewSubmission = false;

  if (existingSubmission) {
    submissionId = existingSubmission.id;
  } else {
    try {
      const [newSubmission] = await tx.transaction(async (savepoint: SubmissionTransaction) =>
        savepoint
          .insert(submissions)
          .values({
            userId,
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
    } catch (creationError) {
      if (!isUniqueConstraintViolation(creationError)) {
        throw creationError;
      }

      const [racedSubmission] = await tx
        .select({ id: submissions.id })
        .from(submissions)
        .where(eq(submissions.userId, userId))
        .for("update")
        .limit(1);

      if (!racedSubmission) {
        throw creationError;
      }

      submissionId = racedSubmission.id;
    }
  }

  const submittedAt = new Date();
  const [submittedDevice] = await tx
    .insert(submittedDevices)
    .values({
      userId,
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
      .for("update");

  let existingDays = await fetchExistingDeviceDays();

  if (
    existingDays.length === 0 &&
    !isNewSubmission &&
    submitDevice.key !== LEGACY_DEVICE_KEY
  ) {
    try {
      await tx.transaction(async (savepoint: SubmissionTransaction) => {
        await savepoint.execute(sql`
          UPDATE daily_breakdown AS db
          SET submitted_device_id = ${submittedDevice.id}
          WHERE db.submission_id = ${submissionId}
            AND db.submitted_device_id IN (
              SELECT sd.id
              FROM submitted_devices AS sd
              WHERE sd.user_id = ${userId}
                AND sd.device_key = ${LEGACY_DEVICE_KEY}
            )
            AND NOT EXISTS (
              SELECT 1
              FROM daily_breakdown AS modern
              WHERE modern.submission_id = db.submission_id
                AND modern.submitted_device_id NOT IN (
                  SELECT sd2.id
                  FROM submitted_devices AS sd2
                  WHERE sd2.user_id = ${userId}
                    AND sd2.device_key = ${LEGACY_DEVICE_KEY}
                )
            )
            AND NOT EXISTS (
              SELECT 1
              FROM daily_breakdown AS duplicate
              WHERE duplicate.submission_id = db.submission_id
                AND duplicate.submitted_device_id = ${submittedDevice.id}
                AND duplicate.date = db.date
            )
        `);
      });
    } catch (adoptionError) {
      if (!isUniqueConstraintViolation(adoptionError)) {
        throw adoptionError;
      }
      console.warn(
        "Legacy adoption conflict (concurrent submit), falling through:",
        adoptionError
      );
    }

    existingDays = await fetchExistingDeviceDays();
  }

  const existingDaysByDate = new Map<string, ExistingDeviceDay>(
    existingDays.map((existingDay: ExistingDeviceDay) => [
      existingDay.date,
      existingDay,
    ])
  );
  const inserts: Array<{
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
  const updates: Array<{
    id: string;
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

    for (const clientContribution of incomingDay.clients) {
      const modelData = clientContributionToBreakdownData(clientContribution);
      const existingClient = incomingClientBreakdown[clientContribution.client];

      if (existingClient) {
        existingClient.tokens += modelData.tokens;
        existingClient.cost += modelData.cost;
        existingClient.input += modelData.input;
        existingClient.output += modelData.output;
        existingClient.cacheRead += modelData.cacheRead;
        existingClient.cacheWrite += modelData.cacheWrite;
        existingClient.reasoning =
          (existingClient.reasoning || 0) + modelData.reasoning;
        existingClient.messages += modelData.messages;

        const existingModel = existingClient.models[clientContribution.modelId];
        if (existingModel) {
          existingModel.tokens += modelData.tokens;
          existingModel.cost += modelData.cost;
          existingModel.input += modelData.input;
          existingModel.output += modelData.output;
          existingModel.cacheRead += modelData.cacheRead;
          existingModel.cacheWrite += modelData.cacheWrite;
          existingModel.reasoning =
            (existingModel.reasoning || 0) + modelData.reasoning;
          existingModel.messages += modelData.messages;
        } else {
          existingClient.models[clientContribution.modelId] = modelData;
        }

        existingClient.provenance =
          deriveClientBreakdownProvenance(existingClient);
      } else {
        const clientBreakdown = {
          ...modelData,
          models: { [clientContribution.modelId]: modelData },
        };
        incomingClientBreakdown[clientContribution.client] = {
          ...clientBreakdown,
          provenance: deriveClientBreakdownProvenance(clientBreakdown),
        };
      }
    }

    const existingDay = existingDaysByDate.get(incomingDay.date);
    if (existingDay) {
      const existingClientBreakdown = (existingDay.sourceBreakdown || {}) as Record<
        string,
        ClientBreakdownData
      >;
      const mergeResult = mergeClientBreakdownsWithRegressionGuard(
        existingClientBreakdown,
        incomingClientBreakdown,
        submittedClients
      );
      warnings.push(
        ...mergeResult.warnings.map(
          (warning) => `Day ${incomingDay.date}: ${warning}`
        )
      );
      const totals = recalculateDayTotals(mergeResult.merged);

      updates.push({
        id: existingDay.id,
        tokens: totals.tokens,
        cost: totals.cost.toFixed(4),
        inputTokens: totals.inputTokens,
        outputTokens: totals.outputTokens,
        timestampMs: mergeTimestampMs(
          existingDay.timestampMs,
          incomingDay.timestampMs ?? null
        ),
        activeTimeMs:
          incomingDay.activeTimeMs ?? existingDay.activeTimeMs ?? null,
        sourceBreakdown: mergeResult.merged,
      });
    } else {
      const totals = recalculateDayTotals(incomingClientBreakdown);
      inserts.push({
        submissionId,
        submittedDeviceId: submittedDevice.id,
        date: incomingDay.date,
        tokens: totals.tokens,
        cost: totals.cost.toFixed(4),
        inputTokens: totals.inputTokens,
        outputTokens: totals.outputTokens,
        timestampMs: incomingDay.timestampMs ?? null,
        activeTimeMs: incomingDay.activeTimeMs ?? null,
        sourceBreakdown: incomingClientBreakdown,
      });
    }
  }

  if (inserts.length > 0) {
    await tx.insert(dailyBreakdown).values(inserts);
  }

  if (updates.length > 0) {
    const values = updates.map(
      (row) =>
        sql`(${row.id}::uuid, ${row.tokens}::bigint, ${row.cost}::numeric(18,4), ${row.inputTokens}::bigint, ${row.outputTokens}::bigint, ${row.timestampMs}::bigint, ${row.activeTimeMs}::bigint, ${JSON.stringify(row.sourceBreakdown)}::jsonb)`
    );

    await tx.execute(sql`
      UPDATE daily_breakdown AS day SET
        tokens = batch.tokens,
        cost = batch.cost,
        input_tokens = batch.input_tokens,
        output_tokens = batch.output_tokens,
        timestamp_ms = batch.timestamp_ms,
        active_time_ms = batch.active_time_ms,
        source_breakdown = batch.source_breakdown
      FROM (VALUES ${sql.join(values, sql`, `)})
        AS batch(id, tokens, cost, input_tokens, output_tokens, timestamp_ms, active_time_ms, source_breakdown)
      WHERE day.id = batch.id
    `);
  }

  const [aggregates] = await tx
    .select({
      totalTokens: sql<number>`COALESCE(SUM(${dailyBreakdown.tokens}), 0)::bigint`,
      totalCost: sql<string>`COALESCE(SUM(CAST(${dailyBreakdown.cost} AS DECIMAL(18,4))), 0)::text`,
      inputTokens: sql<number>`COALESCE(SUM(${dailyBreakdown.inputTokens}), 0)::bigint`,
      outputTokens: sql<number>`COALESCE(SUM(${dailyBreakdown.outputTokens}), 0)::bigint`,
      dateStart: sql<string>`MIN(${dailyBreakdown.date})`,
      dateEnd: sql<string>`MAX(${dailyBreakdown.date})`,
      activeDays: sql<number>`COUNT(DISTINCT CASE WHEN ${dailyBreakdown.tokens} > 0 THEN ${dailyBreakdown.date} END)::int`,
      totalActiveTimeMs: sql<number>`COALESCE(SUM(${dailyBreakdown.activeTimeMs}), 0)::bigint`,
    })
    .from(dailyBreakdown)
    .where(eq(dailyBreakdown.submissionId, submissionId));

  const allDays = await tx
    .select({ sourceBreakdown: dailyBreakdown.sourceBreakdown })
    .from(dailyBreakdown)
    .where(eq(dailyBreakdown.submissionId, submissionId));

  const allClients = new Set<string>();
  const allModels = new Set<string>();
  let totalCacheRead = 0;
  let totalCacheCreation = 0;
  let totalReasoning = 0;

  for (const day of allDays) {
    if (!day.sourceBreakdown) {
      continue;
    }

    for (const [rawClientName, clientData] of Object.entries(
      day.sourceBreakdown
    )) {
      allClients.add(rawClientName === "kilocode" ? "kilo" : rawClientName);
      const breakdown = clientData as ClientBreakdownData;

      if (breakdown.models) {
        for (const modelId of Object.keys(breakdown.models)) {
          allModels.add(modelId);
        }
      } else if (breakdown.modelId) {
        allModels.add(breakdown.modelId);
      }

      totalCacheRead += breakdown.cacheRead || 0;
      totalCacheCreation += breakdown.cacheWrite || 0;
      totalReasoning += breakdown.reasoning || 0;
    }
  }

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
      ...(data.timeMetrics
        ? {
            longestContinuousMs: data.timeMetrics.longestContinuousMs,
            maxConcurrentSessions: data.timeMetrics.maxConcurrentSessions,
            sessionCount: data.timeMetrics.sessionCount,
          }
        : {}),
      mcpServers: mcpServers && mcpServers.length > 0 ? mcpServers : null,
      updatedAt: new Date(),
    })
    .where(eq(submissions.id, submissionId));

  return {
    submissionId,
    isNewSubmission,
    warnings,
    metrics: {
      totalTokens: Number(aggregates.totalTokens ?? 0),
      totalCost: Number.parseFloat(aggregates.totalCost),
      dateRange: {
        start: aggregates.dateStart,
        end: aggregates.dateEnd,
      },
      activeDays: Number(aggregates.activeDays ?? 0),
      clients: Array.from(allClients),
    },
  };
}
