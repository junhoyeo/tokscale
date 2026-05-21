import { NextResponse } from "next/server";
import { getSession } from "@/lib/auth/session";
import { getUserDeviceStats } from "@/lib/db/devices";

/**
 * GET /api/settings/devices
 * List the authenticated user's devices with per-device usage stats.
 */
export async function GET() {
  try {
    const session = await getSession();
    if (!session) {
      return NextResponse.json({ error: "Not authenticated" }, { status: 401 });
    }

    const devices = await getUserDeviceStats(session.id);

    return NextResponse.json({ devices });
  } catch (error) {
    console.error("Devices list error:", error);
    return NextResponse.json(
      { error: "Failed to fetch devices" },
      { status: 500 }
    );
  }
}
