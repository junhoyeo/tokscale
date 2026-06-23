import { NextResponse } from "next/server";
import { revalidateTag } from "next/cache";
import { and, eq } from "drizzle-orm";
import { z } from "zod";
import { db, submittedDevices } from "@/lib/db";
import { getSessionFromRequest } from "@/lib/auth/requestSession";
import { normalizeUsernameCacheKey } from "@/lib/db/usernameLookup";

const UUID_REGEX =
  /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

// 120 chars matches submitted_devices.display_name varchar(120).
// Reject Unicode control characters so the label can't smuggle newlines /
// terminal escape sequences into rendered UI.
const RenameBodySchema = z.object({
  name: z
    .union([z.string(), z.null()])
    .transform((value) => {
      if (value == null) return null;
      const trimmed = value.trim();
      return trimmed === "" ? null : trimmed;
    })
    .refine(
      (value) => value == null || value.length <= 120,
      { message: "name must be 120 characters or fewer" }
    )
    .refine(
      (value) => value == null || !/\p{C}/u.test(value),
      { message: "name must not contain control characters" }
    ),
});

interface RouteParams {
  params: Promise<{ deviceId: string }>;
}

/**
 * PATCH /api/settings/devices/[deviceId]
 *
 * Rename (or clear, with `name: null` / `name: ""`) the display label of a
 * device the caller owns. Ownership is enforced via the session user id +
 * `submitted_devices.user_id` in a single WHERE so a cross-user rename
 * silently 404s without leaking existence.
 */
export async function PATCH(request: Request, { params }: RouteParams) {
  try {
    const session = await getSessionFromRequest(request, {
      allowAuthorizationHeader: false,
    });
    if (!session) {
      return NextResponse.json(
        { error: "Not authenticated" },
        { status: 401 }
      );
    }

    const { deviceId } = await params;
    if (!UUID_REGEX.test(deviceId)) {
      return NextResponse.json(
        { error: "Invalid device id" },
        { status: 400 }
      );
    }

    let body: unknown;
    try {
      body = await request.json();
    } catch {
      return NextResponse.json(
        { error: "Invalid JSON body" },
        { status: 400 }
      );
    }

    const parsed = RenameBodySchema.safeParse(body);
    if (!parsed.success) {
      return NextResponse.json(
        {
          error: "Invalid request body",
          details: parsed.error.issues.map((issue) => issue.message),
        },
        { status: 400 }
      );
    }

    const { name } = parsed.data;
    const updatedAt = new Date();

    const [updated] = await db
      .update(submittedDevices)
      .set({ displayName: name, updatedAt })
      .where(
        and(
          eq(submittedDevices.id, deviceId),
          eq(submittedDevices.userId, session.id)
        )
      )
      .returning({
        id: submittedDevices.id,
        deviceKey: submittedDevices.deviceKey,
        displayName: submittedDevices.displayName,
      });

    if (!updated) {
      return NextResponse.json(
        { error: "Device not found" },
        { status: 404 }
      );
    }

    try {
      // Normalize before building the cache key, matching submit/route.ts:535-540
      // which also calls normalizeUsernameCacheKey before revalidateTag.
      revalidateTag(`user:${normalizeUsernameCacheKey(session.username)}`, "max");
    } catch (e) {
      console.error("Cache invalidation failed after device rename:", e);
    }

    return NextResponse.json({
      success: true,
      device: updated,
    });
  } catch (error) {
    console.error("Device rename error:", error);
    return NextResponse.json(
      { error: "Failed to rename device" },
      { status: 500 }
    );
  }
}
