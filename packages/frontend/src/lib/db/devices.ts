/** Device queries for multi-device usage aggregation. */
import { db, devices, dailyBreakdown, submissions } from "@/lib/db";
import { and, eq, sql, desc } from "drizzle-orm";
import type { ClientBreakdownData } from "@/lib/db/helpers";

/** A drizzle transaction handle, as passed to `db.transaction(async (tx) => ...)`. */
type Tx = Parameters<Parameters<typeof db.transaction>[0]>[0];

export interface DeviceUsageStats {
  id: string;
  deviceId: string;
  name: string;
  hostname: string | null;
  os: string | null;
  cliVersion: string | null;
  lastSeenAt: string | null;
  createdAt: string;
  totalTokens: number;
  totalCost: number;
  lastActiveDate: string | null;
  activeDays: number;
}

function toIso(value: Date | string | null): string | null {
  if (value == null) return null;
  return value instanceof Date ? value.toISOString() : new Date(value).toISOString();
}

/** Lists a user's devices with summed usage, ordered by tokens descending. */
export async function getUserDeviceStats(userId: string): Promise<DeviceUsageStats[]> {
  const rows = await db
    .select({
      id: devices.id,
      deviceId: devices.deviceId,
      name: devices.name,
      hostname: devices.hostname,
      os: devices.os,
      cliVersion: devices.cliVersion,
      lastSeenAt: devices.lastSeenAt,
      createdAt: devices.createdAt,
      totalTokens: sql<number>`COALESCE(SUM(${dailyBreakdown.tokens}), 0)::bigint`,
      totalCost: sql<string>`COALESCE(SUM(CAST(${dailyBreakdown.cost} AS DECIMAL(14,4))), 0)::text`,
      lastActiveDate: sql<string | null>`MAX(${dailyBreakdown.date})`,
      activeDays: sql<number>`COUNT(CASE WHEN ${dailyBreakdown.tokens} > 0 THEN 1 END)::int`,
    })
    .from(devices)
    .leftJoin(dailyBreakdown, eq(dailyBreakdown.deviceId, devices.id))
    .where(eq(devices.userId, userId))
    .groupBy(devices.id)
    .orderBy(desc(sql`COALESCE(SUM(${dailyBreakdown.tokens}), 0)`));

  return rows.map((r) => ({
    id: r.id,
    deviceId: r.deviceId,
    name: r.name,
    hostname: r.hostname,
    os: r.os,
    cliVersion: r.cliVersion,
    lastSeenAt: toIso(r.lastSeenAt),
    createdAt: toIso(r.createdAt) ?? new Date(0).toISOString(),
    totalTokens: Number(r.totalTokens) || 0,
    totalCost: Number(r.totalCost) || 0,
    lastActiveDate: r.lastActiveDate ?? null,
    activeDays: Number(r.activeDays) || 0,
  }));
}

/** Recompute a submission's totals from the SUM of its daily_breakdown rows. */
async function recomputeSubmissionTotals(tx: Tx, submissionId: string): Promise<void> {
  const [aggregates] = await tx
    .select({
      totalTokens: sql<number>`COALESCE(SUM(${dailyBreakdown.tokens}), 0)::bigint`,
      totalCost: sql<string>`COALESCE(SUM(CAST(${dailyBreakdown.cost} AS DECIMAL(12,4))), 0)::text`,
      inputTokens: sql<number>`COALESCE(SUM(${dailyBreakdown.inputTokens}), 0)::bigint`,
      outputTokens: sql<number>`COALESCE(SUM(${dailyBreakdown.outputTokens}), 0)::bigint`,
      dateStart: sql<string | null>`MIN(${dailyBreakdown.date})`,
      dateEnd: sql<string | null>`MAX(${dailyBreakdown.date})`,
      rowCount: sql<number>`COUNT(*)::int`,
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
    if (!day.sourceBreakdown) continue;
    for (const [rawClientName, clientData] of Object.entries(day.sourceBreakdown)) {
      allClients.add(rawClientName === "kilocode" ? "kilo" : rawClientName);
      const cd = clientData as ClientBreakdownData;
      if (cd.models) {
        for (const modelId of Object.keys(cd.models)) allModels.add(modelId);
      } else if (cd.modelId) {
        allModels.add(cd.modelId);
      }
      totalCacheRead += cd.cacheRead || 0;
      totalCacheCreation += cd.cacheWrite || 0;
      totalReasoning += cd.reasoning || 0;
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
      sourcesUsed: Array.from(allClients),
      modelsUsed: Array.from(allModels),
      // Keep the existing date range when no rows remain (columns are NOT NULL).
      ...(aggregates.rowCount > 0 && aggregates.dateStart && aggregates.dateEnd
        ? { dateStart: aggregates.dateStart, dateEnd: aggregates.dateEnd }
        : {}),
      updatedAt: new Date(),
    })
    .where(eq(submissions.id, submissionId));
}

/**
 * Delete one of a user's devices and its usage rows, then recompute the user's
 * submission totals. Returns false if the device is not found for the user.
 */
export async function deleteUserDevice(
  userId: string,
  deviceUuid: string
): Promise<boolean> {
  return db.transaction(async (tx) => {
    const [device] = await tx
      .select({ id: devices.id })
      .from(devices)
      .where(and(eq(devices.id, deviceUuid), eq(devices.userId, userId)))
      .limit(1);

    if (!device) return false;

    // Lock the submission row so totals stay consistent with concurrent submits.
    const [submission] = await tx
      .select({ id: submissions.id })
      .from(submissions)
      .where(eq(submissions.userId, userId))
      .for("update")
      .limit(1);

    // FK onDelete: cascade removes this device's daily_breakdown rows.
    await tx.delete(devices).where(eq(devices.id, device.id));

    if (submission) {
      await recomputeSubmissionTotals(tx, submission.id);
    }

    return true;
  });
}
