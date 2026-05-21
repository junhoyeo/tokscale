import { NextResponse } from "next/server";
import { revalidateTag } from "next/cache";
import { getSession } from "@/lib/auth/session";
import { deleteUserDevice } from "@/lib/db/devices";
import {
  normalizeUsernameCacheKey,
  revalidateUsernamePaths,
} from "@/lib/db/usernameLookup";

interface RouteParams {
  params: Promise<{ deviceId: string }>;
}

const UUID_PATTERN =
  /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

/**
 * DELETE /api/settings/devices/:deviceId
 * Remove one of the authenticated user's devices and all of its usage rows,
 * then recompute the user's submission totals.
 */
export async function DELETE(_request: Request, { params }: RouteParams) {
  try {
    const session = await getSession();
    if (!session) {
      return NextResponse.json({ error: "Not authenticated" }, { status: 401 });
    }

    const { deviceId } = await params;

    if (!UUID_PATTERN.test(deviceId)) {
      return NextResponse.json({ error: "Device not found" }, { status: 404 });
    }

    const deleted = await deleteUserDevice(session.id, deviceId);

    if (!deleted) {
      return NextResponse.json({ error: "Device not found" }, { status: 404 });
    }

    try {
      const usernameCacheKey = normalizeUsernameCacheKey(session.username);
      revalidateTag("leaderboard", "max");
      revalidateTag(`user:${usernameCacheKey}`, "max");
      revalidateTag("user-rank", "max");
      revalidateTag(`user-rank:${usernameCacheKey}`, "max");
      revalidateUsernamePaths(session.username);
    } catch (e) {
      console.error("Cache invalidation failed:", e);
    }

    return NextResponse.json({ success: true });
  } catch (error) {
    console.error("Device delete error:", error);
    return NextResponse.json(
      { error: "Failed to delete device" },
      { status: 500 }
    );
  }
}
