import { NextResponse } from "next/server";
import { asc, desc, eq, sql } from "drizzle-orm";
import { getBearerToken } from "@/lib/auth/bearerToken";
import { authenticatePersonalToken } from "@/lib/auth/personalTokens";
import { db, dailyBreakdown, submissions, submittedDevices } from "@/lib/db";
import { deviceDisplayLabel, toIsoString } from "@/lib/devices/shared";

/**
 * Stable wire contract consumed by the CLI TUI (crates/tokscale-cli/src/tui/remote.rs).
 * Bump `schemaVersion` on breaking changes so old CLIs can detect mismatches.
 */
const SCHEMA_VERSION = 1;

interface MeStatsDay {
  date: string;
  tokens: number;
  inputTokens: number;
  outputTokens: number;
  cost: number;
}

interface MeStatsDevice {
  id: string;
  displayName: string;
  lastSubmittedAt: string | null;
}

interface MeStatsResponse {
  schemaVersion: number;
  totalTokens: number;
  totalCost: number;
  deviceCount: number;
  lastSubmittedAt: string | null;
  days: MeStatsDay[];
  devices: MeStatsDevice[];
}

function emptyStats(): MeStatsResponse {
  return {
    schemaVersion: SCHEMA_VERSION,
    totalTokens: 0,
    totalCost: 0,
    deviceCount: 0,
    lastSubmittedAt: null,
    days: [],
    devices: [],
  };
}

/**
 * GET /api/me/stats
 *
 * Returns the authenticated user's submitted usage aggregated ACROSS all of
 * their devices: per-day totals summed over `daily_breakdown` rows (which are
 * scoped per `submitted_device_id`), overall totals, and the device list with
 * last-submit timestamps.
 *
 * Auth: identical to POST /api/submit — `Authorization: Bearer <api_token>`
 * validated via authenticatePersonalToken. Cookie sessions are intentionally
 * not accepted; this endpoint exists for the CLI.
 *
 * Read-only: performs no writes (touchLastUsedAt is disabled for the same
 * reason as submit's auth path).
 */
export async function GET(request: Request) {
  try {
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
      return NextResponse.json(
        { error: "API token has expired" },
        { status: 401 }
      );
    }

    const [submission] = await db
      .select({ id: submissions.id })
      .from(submissions)
      .where(eq(submissions.userId, authResult.userId))
      .limit(1);

    if (!submission) {
      return NextResponse.json(emptyStats());
    }

    const dayRows = await db
      .select({
        date: dailyBreakdown.date,
        tokens: sql<string>`COALESCE(SUM(${dailyBreakdown.tokens}), 0)`,
        inputTokens: sql<string>`COALESCE(SUM(${dailyBreakdown.inputTokens}), 0)`,
        outputTokens: sql<string>`COALESCE(SUM(${dailyBreakdown.outputTokens}), 0)`,
        cost: sql<string>`COALESCE(SUM(${dailyBreakdown.cost}), 0)`,
      })
      .from(dailyBreakdown)
      .where(eq(dailyBreakdown.submissionId, submission.id))
      .groupBy(dailyBreakdown.date)
      .orderBy(asc(dailyBreakdown.date));

    const deviceRows = await db
      .select({
        id: submittedDevices.id,
        deviceKey: submittedDevices.deviceKey,
        displayName: submittedDevices.displayName,
        lastSubmittedAt: submittedDevices.lastSubmittedAt,
      })
      .from(submittedDevices)
      .where(eq(submittedDevices.userId, authResult.userId))
      .orderBy(
        sql`${submittedDevices.lastSubmittedAt} DESC NULLS LAST`,
        desc(submittedDevices.id)
      );

    const days: MeStatsDay[] = dayRows.map((row) => ({
      date: row.date,
      tokens: Number(row.tokens) || 0,
      inputTokens: Number(row.inputTokens) || 0,
      outputTokens: Number(row.outputTokens) || 0,
      cost: Number(row.cost) || 0,
    }));

    let totalTokens = 0;
    let totalCost = 0;
    for (const day of days) {
      totalTokens += day.tokens;
      totalCost += day.cost;
    }

    const devices: MeStatsDevice[] = deviceRows.map((row) => ({
      id: row.id,
      displayName: deviceDisplayLabel(row.deviceKey, row.displayName),
      lastSubmittedAt: toIsoString(row.lastSubmittedAt),
    }));

    const lastSubmittedAt = devices.reduce<string | null>(
      (latest, device) =>
        device.lastSubmittedAt && (!latest || device.lastSubmittedAt > latest)
          ? device.lastSubmittedAt
          : latest,
      null
    );

    const response: MeStatsResponse = {
      schemaVersion: SCHEMA_VERSION,
      totalTokens,
      totalCost,
      deviceCount: devices.length,
      lastSubmittedAt,
      days,
      devices,
    };

    return NextResponse.json(response);
  } catch (error) {
    console.error("Me stats error:", error);
    return NextResponse.json({ error: "Internal server error" }, { status: 500 });
  }
}
