import { and, desc, eq, ne } from "drizzle-orm";
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
import { revalidateUserGroupLeaderboards } from "@/lib/groups/cache";

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
  actorUsername: string;
  action: ModerationAction;
  reason: string;
}): Promise<{ changed: boolean; leaderboardHidden: boolean }> {
  const { target, actorUserId, actorUsername, action, reason } = params;
  const nextHidden = action === "hide";

  const changed = await db.transaction(async (tx) => {
    const updated = await tx
      .update(users)
      .set({ leaderboardHidden: nextHidden, updatedAt: new Date() })
      // PostgreSQL rechecks this state predicate after concurrent writers
      // commit. Only the transition that actually changed the flag can write
      // the corresponding audit row.
      .where(
        and(
          eq(users.id, target.id),
          ne(users.leaderboardHidden, nextHidden)
        )
      )
      .returning({ id: users.id });

    if (updated.length === 0) {
      return false;
    }

    await tx.insert(moderationActions).values({
      targetUserId: target.id,
      targetUsername: target.username,
      actorUserId,
      actorUsername,
      action,
      reason,
    });

    return true;
  });

  if (changed) {
    await invalidateAfterModeration(target.id, target.username);
  }

  return { changed, leaderboardHidden: nextHidden };
}

/**
 * Drops every cached surface whose contents depend on who is rankable.
 *
 * Best-effort by design: the write has already committed, so a revalidation
 * failure must not surface as a failed moderation action. Worst case the
 * change appears after the existing TTLs expire (60s for the leaderboard and
 * profiles, an hour for the sitemap).
 */
async function invalidateAfterModeration(userId: string, username: string): Promise<void> {
  try {
    // Every leaderboard cache entry carries this tag, including the per-period
    // ones, so one call covers all of them.
    revalidateTag("leaderboard", "max");
    revalidateTag(`user:${normalizeUsernameCacheKey(username)}`, "max");
    revalidateUsernamePaths(username);
    await revalidateUserGroupLeaderboards(userId);
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
  const rows = await db
    .select({
      id: moderationActions.id,
      action: moderationActions.action,
      reason: moderationActions.reason,
      createdAt: moderationActions.createdAt,
      actorUsername: moderationActions.actorUsername,
    })
    .from(moderationActions)
    .where(eq(moderationActions.targetUserId, targetUserId))
    .orderBy(desc(moderationActions.createdAt))
    .limit(limit);

  return rows;
}
