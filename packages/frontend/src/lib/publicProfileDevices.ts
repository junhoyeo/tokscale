import { NextResponse } from "next/server";
import { unstable_cache } from "next/cache";
import { desc, eq, sql } from "drizzle-orm";
import { db, dailyBreakdown, submittedDevices, users } from "@/lib/db";
import {
  USERNAME_LOOKUP_LIMIT,
  getSingleUsernameMatch,
  normalizeUsernameCacheKey,
  usernameEqualsIgnoreCase,
} from "@/lib/db/usernameLookup";
import { deviceDisplayLabel, toIsoString } from "@/lib/devices/shared";

export interface PublicProfileDevicesRouteParams {
  params: Promise<{ username: string }>;
}

/**
 * GET /api/users/[username]/devices
 *
 * Returns the list of submission devices belonging to `username` with usage
 * totals aggregated from `daily_breakdown`. Public — no auth required, mirrors
 * the visibility model of GET /api/users/[username].
 *
 * Returned in `last_submitted_at DESC, created_at DESC` order so the device
 * the user just submitted from is always first.
 */
export async function getPublicProfileDevicesResponse(
  _request: Request,
  { params }: PublicProfileDevicesRouteParams,
) {
  try {
    const { username } = await params;

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

    const rows = await db
      .select({
        id: submittedDevices.id,
        deviceKey: submittedDevices.deviceKey,
        displayName: submittedDevices.displayName,
        createdAt: submittedDevices.createdAt,
        lastSubmittedAt: submittedDevices.lastSubmittedAt,
        totalTokens: sql<string>`COALESCE(SUM(${dailyBreakdown.tokens}), 0)`,
        totalCost: sql<string>`COALESCE(SUM(${dailyBreakdown.cost}), 0)`,
        inputTokens: sql<string>`COALESCE(SUM(${dailyBreakdown.inputTokens}), 0)`,
        outputTokens: sql<string>`COALESCE(SUM(${dailyBreakdown.outputTokens}), 0)`,
        activeDays: sql<string>`COUNT(DISTINCT CASE WHEN ${dailyBreakdown.tokens} > 0 THEN ${dailyBreakdown.date} END)`,
        firstDay: sql<string | null>`MIN(${dailyBreakdown.date})`,
        lastDay: sql<string | null>`MAX(${dailyBreakdown.date})`,
      })
      .from(submittedDevices)
      .leftJoin(dailyBreakdown, eq(dailyBreakdown.submittedDeviceId, submittedDevices.id))
      .where(eq(submittedDevices.userId, user.id))
      .groupBy(
        submittedDevices.id,
        submittedDevices.deviceKey,
        submittedDevices.displayName,
        submittedDevices.createdAt,
        submittedDevices.lastSubmittedAt
      )
      .orderBy(
        sql`${submittedDevices.lastSubmittedAt} DESC NULLS LAST`,
        desc(submittedDevices.createdAt)
      );

    return NextResponse.json({
      user: {
        username: user.username,
        displayName: user.displayName,
        avatarUrl: user.avatarUrl,
      },
      devices: rows.map((row) => ({
        id: row.id,
        deviceKey: row.deviceKey,
        // Public consumers render the resolved fallback label; `customName`
        // carries the raw nullable value so owners (settings UI) can
        // distinguish "user typed this" from "fallback label".
        displayName: deviceDisplayLabel(row.deviceKey, row.displayName),
        customName: row.displayName,
        createdAt: toIsoString(row.createdAt),
        lastSubmittedAt: toIsoString(row.lastSubmittedAt),
        totalTokens: Number(row.totalTokens) || 0,
        totalCost: Number(row.totalCost) || 0,
        inputTokens: Number(row.inputTokens) || 0,
        outputTokens: Number(row.outputTokens) || 0,
        activeDays: Number(row.activeDays) || 0,
        firstDay: row.firstDay,
        lastDay: row.lastDay,
      })),
    });
  } catch (error) {
    console.error("Get user devices error:", error);
    return NextResponse.json(
      { error: "Failed to fetch devices" },
      { status: 500 }
    );
  }
}

export async function loadPublicProfileDevicesForPage(
  username: string,
): Promise<unknown[]> {
  const load = unstable_cache(
    async (): Promise<unknown[]> => {
      const response = await getPublicProfileDevicesResponse(
        new Request(
          `http://tokscale.internal/api/users/${encodeURIComponent(username)}/devices`,
        ),
        { params: Promise.resolve({ username }) },
      );
      if (!response.ok) return [];

      const data = (await response.json()) as { devices?: unknown };
      return Array.isArray(data.devices) ? data.devices : [];
    },
    ["public-profile-devices-page", username],
    {
      revalidate: 60,
      tags: [`user:${normalizeUsernameCacheKey(username)}`],
    },
  );

  return load();
}
