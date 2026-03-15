import { NextResponse } from "next/server";
import { eq } from "drizzle-orm";
import { authenticatePersonalToken } from "@/lib/auth/personalTokens";
import { db, dailyBreakdown, submissions } from "@/lib/db";
import type { ClientBreakdownData as DbClientBreakdownData } from "@/lib/db/helpers";

type ClientBreakdownData = Omit<DbClientBreakdownData, "modelId"> & {
  modelId?: string;
};

type LegacySourceBreakdown = Record<string, ClientBreakdownData>;
type DeviceSourceBreakdown = {
  devices: Record<string, Record<string, ClientBreakdownData>>;
};

type DailyRow = {
  date: string;
  tokens: number;
  cost: string | number;
  timestampMs: number | null;
  sourceBreakdown: LegacySourceBreakdown | DeviceSourceBreakdown | null;
};

type StatsResponse = {
  totalCost: number;
  totalTokens: number;
  byModel: Array<{ model: string; cost: number; tokens: number }>;
  byDay: Array<{ date: string; cost: number; tokens: number }>;
  byClient: Array<{ client: string; cost: number; tokens: number }>;
  devices: Array<{ id: string; lastSeenAt?: string; cost: number }>;
};

const EMPTY_STATS: StatsResponse = {
  totalCost: 0,
  totalTokens: 0,
  byModel: [],
  byDay: [],
  byClient: [],
  devices: [],
};

function isRecord(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === "object";
}

function hasDevicesBreakdown(
  value: LegacySourceBreakdown | DeviceSourceBreakdown | null | undefined
): value is DeviceSourceBreakdown {
  return isRecord(value) && isRecord(value.devices);
}

function addClientAggregates(
  clientTotals: Map<string, { cost: number; tokens: number }>,
  modelTotals: Map<string, { cost: number; tokens: number }>,
  clientName: string,
  clientData: ClientBreakdownData
) {
  const tokens = clientData.tokens || 0;
  const cost = clientData.cost || 0;
  const existingClient = clientTotals.get(clientName);

  if (existingClient) {
    existingClient.tokens += tokens;
    existingClient.cost += cost;
  } else {
    clientTotals.set(clientName, { tokens, cost });
  }

  if (clientData.models && Object.keys(clientData.models).length > 0) {
    for (const [model, modelData] of Object.entries(clientData.models)) {
      const existingModel = modelTotals.get(model);
      if (existingModel) {
        existingModel.tokens += modelData.tokens || 0;
        existingModel.cost += modelData.cost || 0;
      } else {
        modelTotals.set(model, {
          tokens: modelData.tokens || 0,
          cost: modelData.cost || 0,
        });
      }
    }
    return;
  }

  const legacyModelId = (clientData as { modelId?: string }).modelId;
  if (!legacyModelId) return;

  const existingModel = modelTotals.get(legacyModelId);
  if (existingModel) {
    existingModel.tokens += tokens;
    existingModel.cost += cost;
  } else {
    modelTotals.set(legacyModelId, { tokens, cost });
  }
}

function buildStats(rows: DailyRow[]): StatsResponse {
  if (rows.length === 0) return EMPTY_STATS;

  let totalCost = 0;
  let totalTokens = 0;
  const byDay: StatsResponse["byDay"] = [];
  const byClient = new Map<string, { cost: number; tokens: number }>();
  const byModel = new Map<string, { cost: number; tokens: number }>();
  const devices = new Map<string, { cost: number; lastSeenTimestampMs: number | null }>();

  for (const row of rows) {
    const dayCost = Number(row.cost) || 0;
    const dayTokens = row.tokens || 0;

    totalCost += dayCost;
    totalTokens += dayTokens;
    byDay.push({
      date: row.date,
      cost: dayCost,
      tokens: dayTokens,
    });

    if (!row.sourceBreakdown) continue;

    if (hasDevicesBreakdown(row.sourceBreakdown)) {
      for (const [deviceId, clientBreakdown] of Object.entries(row.sourceBreakdown.devices)) {
        let deviceDayCost = 0;

        for (const [clientName, clientData] of Object.entries(clientBreakdown)) {
          addClientAggregates(byClient, byModel, clientName, clientData);
          deviceDayCost += clientData.cost || 0;
        }

        const existingDevice = devices.get(deviceId);
        const lastSeenTimestampMs = row.timestampMs ?? null;
        if (existingDevice) {
          existingDevice.cost += deviceDayCost;
          if (
            lastSeenTimestampMs != null &&
            (existingDevice.lastSeenTimestampMs == null ||
              lastSeenTimestampMs > existingDevice.lastSeenTimestampMs)
          ) {
            existingDevice.lastSeenTimestampMs = lastSeenTimestampMs;
          }
        } else {
          devices.set(deviceId, {
            cost: deviceDayCost,
            lastSeenTimestampMs,
          });
        }
      }

      continue;
    }

    for (const [clientName, clientData] of Object.entries(row.sourceBreakdown)) {
      addClientAggregates(byClient, byModel, clientName, clientData);
    }
  }

  return {
    totalCost,
    totalTokens,
    byModel: Array.from(byModel.entries())
      .map(([model, totals]) => ({
        model,
        cost: totals.cost,
        tokens: totals.tokens,
      }))
      .sort((a, b) => b.tokens - a.tokens || a.model.localeCompare(b.model)),
    byDay,
    byClient: Array.from(byClient.entries())
      .map(([client, totals]) => ({
        client,
        cost: totals.cost,
        tokens: totals.tokens,
      }))
      .sort((a, b) => b.tokens - a.tokens || a.client.localeCompare(b.client)),
    devices: Array.from(devices.entries())
      .map(([id, device]) => ({
        id,
        cost: device.cost,
        ...(device.lastSeenTimestampMs != null
          ? { lastSeenAt: new Date(device.lastSeenTimestampMs).toISOString() }
          : {}),
      }))
      .sort((a, b) => b.cost - a.cost || a.id.localeCompare(b.id)),
  };
}

export async function GET(request: Request) {
  try {
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

    const [submission] = await db
      .select({ id: submissions.id })
      .from(submissions)
      .where(eq(submissions.userId, authResult.userId))
      .limit(1);

    if (!submission) {
      return NextResponse.json(EMPTY_STATS);
    }

    const rows = await db
      .select({
        date: dailyBreakdown.date,
        tokens: dailyBreakdown.tokens,
        cost: dailyBreakdown.cost,
        timestampMs: dailyBreakdown.timestampMs,
        sourceBreakdown: dailyBreakdown.sourceBreakdown,
      })
      .from(dailyBreakdown)
      .where(eq(dailyBreakdown.submissionId, submission.id))
      .orderBy(dailyBreakdown.date);

    return NextResponse.json(buildStats(rows as DailyRow[]));
  } catch (error) {
    console.error("Me stats error:", error);
    return NextResponse.json({ error: "Internal server error" }, { status: 500 });
  }
}
