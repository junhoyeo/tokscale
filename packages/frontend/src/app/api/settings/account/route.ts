import { revalidatePath, revalidateTag } from "next/cache";
import { NextResponse } from "next/server";
import { eq } from "drizzle-orm";
import { clearSession } from "@/lib/auth/session";
import { getSessionFromRequest } from "@/lib/auth/requestSession";
import { db, users } from "@/lib/db";
import {
  normalizeUsernameCacheKey,
  revalidateUsernamePaths,
} from "@/lib/db/usernameLookup";
import { revalidateUserGroupLeaderboards } from "@/lib/groups/cache";

export async function DELETE(request: Request) {
  try {
    // Session-cookie auth routed through getSessionFromRequest so the CSRF
    // Origin allowlist applies — the same gate every other cookie-mutating
    // settings route uses. Account deletion deliberately has no Bearer
    // personal-token path: it is a web Settings UI action, so session-only
    // auth is the conservative default.
    const session = await getSessionFromRequest(request);
    if (!session) {
      return NextResponse.json({ error: "Not authenticated" }, { status: 401 });
    }

    const username = session.username;
    const usernameCacheKey = normalizeUsernameCacheKey(username);

    // Group leaderboard invalidation must run BEFORE the user row delete:
    // the helper resolves the user's group membership rows, and the delete
    // below cascades those rows away. Best-effort like all revalidation.
    try {
      await revalidateUserGroupLeaderboards(session.id);
    } catch (cacheError) {
      console.error(
        "Group cache invalidation failed before account deletion:",
        cacheError
      );
    }

    // Delete the user row — all related data (sessions, apiTokens,
    // submissions → dailyBreakdown, deviceCodes, group memberships)
    // cascades automatically via ON DELETE CASCADE foreign keys.
    const deletedRows = await db
      .delete(users)
      .where(eq(users.id, session.id))
      .returning({ id: users.id });

    if (deletedRows.length === 0) {
      return NextResponse.json({ error: "User not found" }, { status: 404 });
    }

    // Clear the session cookie so the browser doesn't hold a stale token.
    // The DB session row is already gone via cascade, but the cookie
    // still needs explicit removal.
    try {
      await clearSession();
    } catch {
      // Cookie cleanup is best-effort — the session row is already gone.
    }

    try {
      revalidateTag("leaderboard", "max");
      revalidateTag(`user:${usernameCacheKey}`, "max");
      revalidateTag("user-rank", "max");
      revalidateTag(`user-rank:${usernameCacheKey}`, "max");
      revalidateTag(`embed-user:${usernameCacheKey}`, "max");
      revalidateTag(`embed-user:${usernameCacheKey}:tokens`, "max");
      revalidateTag(`embed-user:${usernameCacheKey}:cost`, "max");

      revalidatePath("/leaderboard");
      revalidatePath("/profile");
      revalidateUsernamePaths(username);
    } catch {
      // Cache invalidation is best-effort.
    }

    return NextResponse.json({ success: true, deleted: true });
  } catch (error) {
    console.error("Account delete error:", error);
    return NextResponse.json(
      { error: "Failed to delete account" },
      { status: 500 }
    );
  }
}
