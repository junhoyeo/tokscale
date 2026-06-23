import { NextResponse } from "next/server";
import { and, desc, eq, sql } from "drizzle-orm";
// `and` is used to enforce ownership and id match in a single WHERE.
import { db, dailyBreakdown, submittedDevices, users } from "@/lib/db";
import {
  USERNAME_LOOKUP_LIMIT,
  getSingleUsernameMatch,
  usernameEqualsIgnoreCase,
} from "@/lib/db/usernameLookup";
import { deviceDisplayLabel, toIsoString } from "@/lib/devices/shared";

export const revalidate = 60;

const UUID_REGEX =
  /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

interface RouteParams {
  params: Promise<{ username: string; deviceId: string }>;
}

/**
 * GET /api/users/[username]/devices/[deviceId]
 *
 * Per-device detail: device metadata + ordered list of daily contributions
 * for that one device. Public read — matches the rest of the user-profile
 * read APIs (auth is only required for mutating actions).
 *
 * `deviceId` must be a valid uuid. Reject other shapes early so we don't even
 * touch the DB for obviously bogus input.
 */
export async function GET(_request: Request, { params }: RouteParams) {
  try {
    const { username, deviceId } = await params;

    if (!UUID_REGEX.test(deviceId)) {
      return NextResponse.json(
        { error: "Invalid device id" },
        { status: 400 }
      );
    }

    const matchingUsers = await db
      .select({
        id: users.id,
        username: users.username,
        displayName: users.displayName,
        avatarUrl: users.avatarUrl,
      })
      .from(users)
      .where(usernameEqualsIgnoreCase(username))
      .limit(USERNAME_LOOKUP_LIMIT);

    const user = getSingleUsernameMatch(matchingUsers, username);
    if (!user) {
      return NextResponse.json({ error: "User not found" }, { status: 404 });
    }

    // Resolve device but check user ownership in the same WHERE so we never
    // leak another user's device id via 404-vs-403 timing.
    const [device] = await db
      .select({
        id: submittedDevices.id,
        deviceKey: submittedDevices.deviceKey,
        displayName: submittedDevices.displayName,
        createdAt: submittedDevices.createdAt,
        lastSubmittedAt: submittedDevices.lastSubmittedAt,
      })
      .from(submittedDevices)
      .where(
        and(
          eq(submittedDevices.id, deviceId),
          eq(submittedDevices.userId, user.id)
        )
      )
      .limit(1);

    if (!device) {
      return NextResponse.json({ error: "Device not found" }, { status: 404 });
    }

    const dailyRows = await db
      .select({
        date: dailyBreakdown.date,
        tokens: dailyBreakdown.tokens,
        cost: dailyBreakdown.cost,
        inputTokens: dailyBreakdown.inputTokens,
        outputTokens: dailyBreakdown.outputTokens,
        timestampMs: dailyBreakdown.timestampMs,
      })
      .from(dailyBreakdown)
      .where(eq(dailyBreakdown.submittedDeviceId, device.id))
      .orderBy(desc(dailyBreakdown.date));

    // Aggregate totals in SQL, matching the pattern in the listing endpoint
    // (src/app/api/users/[username]/devices/route.ts), rather than summing in JS.
    const [totalsRow] = await db
      .select({
        totalTokens: sql<string>`COALESCE(SUM(${dailyBreakdown.tokens}), 0)`,
        totalCost: sql<string>`COALESCE(SUM(${dailyBreakdown.cost}), 0)`,
        inputTokens: sql<string>`COALESCE(SUM(${dailyBreakdown.inputTokens}), 0)`,
        outputTokens: sql<string>`COALESCE(SUM(${dailyBreakdown.outputTokens}), 0)`,
        activeDays: sql<string>`COUNT(DISTINCT CASE WHEN ${dailyBreakdown.tokens} > 0 THEN ${dailyBreakdown.date} END)`,
      })
      .from(dailyBreakdown)
      .where(eq(dailyBreakdown.submittedDeviceId, device.id));

    const totalTokens = Number(totalsRow?.totalTokens) || 0;
    const totalCost = Number(totalsRow?.totalCost) || 0;
    const inputTokens = Number(totalsRow?.inputTokens) || 0;
    const outputTokens = Number(totalsRow?.outputTokens) || 0;
    const activeDays = Number(totalsRow?.activeDays) || 0;

    return NextResponse.json({
      user: {
        username: user.username,
        displayName: user.displayName,
        avatarUrl: user.avatarUrl,
      },
      device: {
        id: device.id,
        deviceKey: device.deviceKey,
        displayName: deviceDisplayLabel(device.deviceKey, device.displayName),
        createdAt: toIsoString(device.createdAt),
        lastSubmittedAt: toIsoString(device.lastSubmittedAt),
      },
      totals: {
        totalTokens,
        totalCost,
        inputTokens,
        outputTokens,
        activeDays,
      },
      contributions: dailyRows.map((row) => ({
        date: row.date,
        tokens: Number(row.tokens) || 0,
        cost: Number(row.cost) || 0,
        inputTokens: Number(row.inputTokens) || 0,
        outputTokens: Number(row.outputTokens) || 0,
        timestampMs: row.timestampMs == null ? null : Number(row.timestampMs),
      })),
    });
  } catch (error) {
    console.error("Get user device detail error:", error);
    return NextResponse.json(
      { error: "Failed to fetch device" },
      { status: 500 }
    );
  }
}
