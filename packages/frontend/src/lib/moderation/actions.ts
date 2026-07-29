import { desc, eq } from "drizzle-orm";
import { revalidatePath, revalidateTag } from "next/cache";
import { db, moderationActions, users } from "@/lib/db";
import {
  USERNAME_LOOKUP_LIMIT,
  getSingleUsernameMatch,
  normalizeUsernameCacheKey,
  revalidateUsernamePaths,
  usernameEqualsIgnoreCase,
} from "@/lib/db/usernameLookup";
import type { ModerationAction } from "@/lib/db/schema";

export interface ModerationTarget {
  id: string;
  username: string;
  leaderboardHidden: boolean;
}

export interface ModerationHistoryEntry {
  id: string;
  action: ModerationAction;
  reason: string;
  createdAt: Date;
  actorUsername: string | null;
}

export async function findModerationTarget(
  username: string
): Promise<ModerationTarget | null> {
  const rows = await db
    .select({
      id: users.id,
      username: users.username,
      leaderboardHidden: users.leaderboardHidden,
    })
    .from(users)
    .where(usernameEqualsIgnoreCase(username))
    .limit(USERNAME_LOOKUP_LIMIT);

  return getSingleUsernameMatch(rows, username);
}

/**
 * Applies a hide/unhide and records why, in one transaction.
 *
 * The flag and the audit row are written together deliberately: a flag with no
 * recorded reason is exactly the situation that makes a moderation decision
 * impossible to defend or reverse later.
 *
 * Idempotent. Re-applying the current state is a no-op that writes no audit
 * row, so a double-clicked button does not litter the history with entries
 * that changed nothing.
 */
export async function applyModerationAction(params: {
  target: ModerationTarget;
  actorUserId: string;
  action: ModerationAction;
  reason: string;
}): Promise<{ changed: boolean; leaderboardHidden: boolean }> {
  const { target, actorUserId, action, reason } = params;
  const nextHidden = action === "hide";

  if (target.leaderboardHidden === nextHidden) {
    return { changed: false, leaderboardHidden: target.leaderboardHidden };
  }

  await db.transaction(async (tx) => {
    await tx
      .update(users)
      .set({ leaderboardHidden: nextHidden, updatedAt: new Date() })
      .where(eq(users.id, target.id));

    await tx.insert(moderationActions).values({
      targetUserId: target.id,
      actorUserId,
      action,
      reason,
    });
  });

  invalidateAfterModeration(target.username);

  return { changed: true, leaderboardHidden: nextHidden };
}

/**
 * Drops every cached surface whose contents depend on who is rankable.
 *
 * Best-effort by design: the write has already committed, so a revalidation
 * failure must not surface as a failed moderation action. Worst case the
 * change appears after the existing TTLs expire (60s for the leaderboard and
 * profiles, an hour for the sitemap).
 */
function invalidateAfterModeration(username: string): void {
  try {
    // Every leaderboard cache entry carries this tag, including the per-period
    // ones, so one call covers all of them.
    revalidateTag("leaderboard", "max");
    revalidateTag(`user:${normalizeUsernameCacheKey(username)}`, "max");
    revalidateUsernamePaths(username);
    // The landing page renders its own top-5 outside the leaderboard cache.
    revalidatePath("/");
    revalidatePath("/leaderboard");
  } catch (error) {
    console.error("[moderation] cache revalidation failed:", error);
  }
}

export async function getModerationHistory(
  targetUserId: string,
  limit = 20
): Promise<ModerationHistoryEntry[]> {
  // Joins users once, on the actor. No alias needed: the target is already
  // pinned by the WHERE clause rather than joined.
  const rows = await db
    .select({
      id: moderationActions.id,
      action: moderationActions.action,
      reason: moderationActions.reason,
      createdAt: moderationActions.createdAt,
      actorUsername: users.username,
    })
    .from(moderationActions)
    .innerJoin(users, eq(moderationActions.actorUserId, users.id))
    .where(eq(moderationActions.targetUserId, targetUserId))
    .orderBy(desc(moderationActions.createdAt))
    .limit(limit);

  return rows;
}
