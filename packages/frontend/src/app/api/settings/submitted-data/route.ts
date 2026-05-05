import { revalidatePath, revalidateTag } from "next/cache";
import { NextResponse } from "next/server";
import { eq } from "drizzle-orm";
import { getSession } from "@/lib/auth/session";
import { authenticatePersonalToken } from "@/lib/auth/personalTokens";
import { db, submissions } from "@/lib/db";
import { normalizeUsernameCacheKey, revalidateUsernamePaths } from "@/lib/db/usernameLookup";
import { getBearerToken } from "../../../../lib/auth/bearerToken";

async function resolveUser(request: Request): Promise<{ id: string; username: string } | null> {
  const token = getBearerToken(request.headers.get("Authorization"));
  if (token) {
    const result = await authenticatePersonalToken(token, { touchLastUsedAt: false });
    if (result.status === "valid") {
      return { id: result.userId, username: result.username };
    }
    return null;
  }

  const session = await getSession();
  if (session) {
    return { id: session.id, username: session.username };
  }
  return null;
}

export async function DELETE(request: Request) {
  try {
    const user = await resolveUser(request);
    if (!user) {
      return NextResponse.json({ error: "Not authenticated" }, { status: 401 });
    }

    const deletedRows = await db
      .delete(submissions)
      .where(eq(submissions.userId, user.id))
      .returning({ id: submissions.id });

    try {
      const usernameCacheKey = normalizeUsernameCacheKey(user.username);

      revalidateTag("leaderboard", "max");
      revalidateTag(`user:${usernameCacheKey}`, "max");
      revalidateTag("user-rank", "max");
      revalidateTag(`user-rank:${usernameCacheKey}`, "max");
      revalidateTag(`embed-user:${usernameCacheKey}`, "max");
      revalidateTag(`embed-user:${usernameCacheKey}:tokens`, "max");
      revalidateTag(`embed-user:${usernameCacheKey}:cost`, "max");

      revalidatePath("/leaderboard");
      revalidatePath("/profile");
      revalidateUsernamePaths(user.username);
    } catch (cacheError) {
      console.error("Cache invalidation failed after deletion:", cacheError);
    }

    return NextResponse.json({
      success: true,
      deleted: deletedRows.length > 0,
      deletedSubmissions: deletedRows.length,
    });
  } catch (error) {
    console.error("Submitted data delete error:", error);
    return NextResponse.json(
      { error: "Failed to delete submitted usage data" },
      { status: 500 }
    );
  }
}
