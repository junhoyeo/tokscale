import { and, desc, eq } from "drizzle-orm";
import { db, moderationActions, users } from "@/lib/db";

/**
 * The notice a hidden user sees on their own profile.
 *
 * Deliberately NOT part of publicProfileData: that response is unstable_cache'd
 * and served to everyone, so putting moderation state in it would both leak the
 * decision publicly and pin it in a shared cache entry. This is fetched
 * separately, uncached, and only after the viewer has been confirmed as the
 * account owner.
 */
export interface ModerationNotice {
  tone: "enforcement" | "pending" | "our-fault";
  message: string;
}

const CONTACT_EMAIL = "i@junho.io";

/**
 * Maps a stored reason to what the account owner is told.
 *
 * Kept pure so the wording is testable without a database, and split by tone
 * because the reasons are not morally equivalent. "Data issue on our side"
 * means our own ratchet inflation (#960) removed them — telling that person
 * they were caught abusing the leaderboard would be a false accusation, and
 * they have nothing to appeal.
 *
 * None of these say how the account was identified. That stays out of anything
 * user-visible for the same reason the stored reasons are vague.
 */
export function moderationNoticeFor(reason: string | null): ModerationNotice {
  if (reason === "Data issue on our side") {
    return {
      tone: "our-fault",
      message:
        `Your usage is temporarily hidden from the leaderboard because of a data problem on our side — not anything you did. ` +
        `Your profile and totals are unaffected, and it will be restored once the underlying issue is corrected. ` +
        `Questions: ${CONTACT_EMAIL}`,
    };
  }

  if (reason === "Abuse") {
    return {
      tone: "enforcement",
      message:
        `Your account has been removed from the leaderboard for abusing it. ` +
        `Your profile and totals remain public, but you no longer hold a ranking position. ` +
        `If you believe this is a mistake, email ${CONTACT_EMAIL}`,
    };
  }

  // Everything else, including a missing reason, gets the neutral wording.
  // Only an explicit "Abuse" accuses anyone: if the flag was set without an
  // audit row, or under a reason added later, we do not actually know what
  // happened — and "you abused this" is the one message that is unfair to send
  // on a guess. This wording is true in every case where someone is hidden.
  return {
    tone: "pending",
    message:
      `Your account is currently withheld from the leaderboard pending review. ` +
      `Your profile, badges and totals still work. ` +
      `If you think this is a mistake, email ${CONTACT_EMAIL}`,
  };
}

/**
 * Returns the notice for `userId`, or null when they are not hidden.
 *
 * The caller MUST have already established that the requesting session owns
 * this account. This performs no authorization of its own.
 */
export async function getModerationNotice(
  userId: string
): Promise<ModerationNotice | null> {
  const [row] = await db
    .select({ leaderboardHidden: users.leaderboardHidden })
    .from(users)
    .where(eq(users.id, userId))
    .limit(1);

  if (!row?.leaderboardHidden) {
    return null;
  }

  // The most recent hide explains the current state; earlier entries are
  // superseded history.
  const [latest] = await db
    .select({ reason: moderationActions.reason })
    .from(moderationActions)
    .where(
      and(
        eq(moderationActions.targetUserId, userId),
        eq(moderationActions.action, "hide")
      )
    )
    .orderBy(desc(moderationActions.createdAt))
    .limit(1);

  return moderationNoticeFor(latest?.reason ?? null);
}
