import { NextResponse } from "next/server";
import { getRecentUsage, RECENT_USAGE_MAX_USERS } from "@/lib/leaderboard/recentUsage";

const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

/**
 * GET /api/leaderboard/recent?users=<uuid>,<uuid>…
 *
 * Daily tokens over the last 30 UTC days for the users on one leaderboard
 * page, for its "Past 30 days" column. Only ids that are UUIDs are read, at
 * most RECENT_USAGE_MAX_USERS of them; the series are the same public daily
 * totals the profiles already show.
 */
export async function GET(request: Request) {
  const { searchParams } = new URL(request.url);
  const ids = (searchParams.get("users") ?? "")
    .split(",")
    .map((id) => id.trim())
    .filter((id) => UUID.test(id));

  if (ids.length === 0) {
    return NextResponse.json({ error: "Pass one or more user ids in ?users=" }, { status: 400 });
  }
  if (ids.length > RECENT_USAGE_MAX_USERS) {
    return NextResponse.json({ error: `At most ${RECENT_USAGE_MAX_USERS} users per request` }, { status: 400 });
  }

  try {
    const usage = await getRecentUsage(ids);
    return NextResponse.json(usage, {
      headers: { "Cache-Control": "public, s-maxage=300, stale-while-revalidate=600" },
    });
  } catch (error) {
    console.error("Leaderboard recent usage error:", error);
    return NextResponse.json({ error: "Failed to fetch recent usage" }, { status: 500 });
  }
}
