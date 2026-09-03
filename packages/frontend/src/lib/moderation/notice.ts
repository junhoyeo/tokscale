import { and, desc, eq } from "drizzle-orm";
import { db, moderationActions, users } from "@/lib/db";

/**
 * Who is reading the notice.
 *
 * The same moderation state is stated the same way to both, except for how the
 * account itself is referred to. The owner is being told something about their
 * own account; a visitor is being told why a profile they are looking at is
 * missing from the leaderboard. So every possessive moves to third person —
 * "Your profile and totals" shown to a visitor claims the account belongs to
 * whoever is reading it.
 *
 * The appeal clause is not part of that substitution and is meant to stay in
 * both. Its "you" addresses whoever spotted a wrong call, which is true of any
 * reader; a visitor handed a verdict with no way to report it wrong is the
 * thing that would be worse.
 */
export type ModerationAudience = "owner" | "public";

/**
 * The notice shown on a hidden user's profile.
 *
 * Deliberately NOT part of publicProfileData: that response is unstable_cache'd
 * and also serves /api/users/<username>, so moderation state travelling with it
 * would put the decision in the public JSON API as a side effect of putting it
 * on the page. This is fetched separately and uncached, which costs one
 * primary-key lookup per profile view — getModerationNotice returns before
 * touching moderation_actions for everyone who is not hidden.
 */
export interface ModerationNotice {
  tone: "enforcement" | "pending" | "our-fault";
  /**
   * Lives here rather than in the page component because it varies by audience
   * for the same tone, and splitting title and body across two files is how
   * they drift into disagreeing about who is being addressed.
   */
  title: string;
  message: string;
}

const CONTACT_EMAIL = "i@junho.io";

/**
 * Maps a stored reason to what the reader is told.
 *
 * Kept pure so the wording is testable without a database, and split by tone
 * because the reasons are not morally equivalent. "Data issue on our side"
 * means our own ratchet inflation (#960) removed them — telling that person
 * they were caught abusing the leaderboard would be a false accusation, and
 * they have nothing to appeal.
 *
 * The two audiences say the same things in the same order. Only the way the
 * account is referred to changes: second person for the owner, third for
 * everyone else. Shortening the public version was tried and dropped — a
 * visitor who is told an account was removed and nothing else has been handed
 * a verdict with no way to tell us it is wrong, and the clauses that were cut
 * ("no longer holds a ranking position", "rank badges show N/A", "will be
 * restored once the underlying issue is corrected") are the ones that explain
 * what the state actually means. The contact address stays for the same
 * reason: a wrong call is worth hearing about from whoever notices it.
 *
 * None of these say how the account was identified. That stays out of anything
 * user-visible, and it matters more in the public wording than the private
 * one: naming the signal in front of everyone is a published guide to evading
 * it.
 */
export function moderationNoticeFor(
  reason: string | null,
  audience: ModerationAudience = "owner"
): ModerationNotice {
  if (reason === "Data issue on our side") {
    return audience === "public"
      ? {
          tone: "our-fault",
          title: "Temporarily hidden — our issue, not theirs",
          message:
            `This account's usage is temporarily hidden from the leaderboard because of a data problem on our side — not anything the account owner did. ` +
            `Its profile and totals are unaffected, and it will be restored once the underlying issue is corrected. ` +
            `Questions: ${CONTACT_EMAIL}`,
        }
      : {
          tone: "our-fault",
          title: "Temporarily hidden — our issue, not yours",
          message:
            `Your usage is temporarily hidden from the leaderboard because of a data problem on our side — not anything you did. ` +
            `Your profile and totals are unaffected, and it will be restored once the underlying issue is corrected. ` +
            `Questions: ${CONTACT_EMAIL}`,
        };
  }

  if (reason === "Abuse") {
    return audience === "public"
      ? {
          tone: "enforcement",
          title: "Removed from the leaderboard",
          message:
            `This account has been removed from the leaderboard for abusing it. ` +
            `Its profile and totals remain public, but it no longer holds a ranking position. ` +
            `If you believe this is a mistake, email ${CONTACT_EMAIL}`,
        }
      : {
          tone: "enforcement",
          title: "Removed from the leaderboard",
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
  return audience === "public"
    ? {
        tone: "pending",
        title: "Withheld from the leaderboard",
        message:
          `This account is currently withheld from the leaderboard pending review. ` +
          `Its profile and totals still work, but rank badges show N/A while it is withheld. ` +
          `If you think this is a mistake, email ${CONTACT_EMAIL}`,
      }
    : {
        tone: "pending",
        title: "Withheld from the leaderboard",
        message:
          `Your account is currently withheld from the leaderboard pending review. ` +
          `Your profile and totals still work, but rank badges show N/A while your account is withheld. ` +
          `If you think this is a mistake, email ${CONTACT_EMAIL}`,
      };
}

/**
 * Returns the notice for `userId`, or null when they are not hidden.
 *
 * Callers pass the audience rather than proving ownership: the notice is shown
 * to everyone, and the only thing ownership decides is which wording. Passing
 * "owner" to a viewer who is not one tells a stranger the account is theirs,
 * so any call site that has not established ownership must pass "public".
 */
export async function getModerationNotice(
  userId: string,
  audience: ModerationAudience = "owner"
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

  return moderationNoticeFor(latest?.reason ?? null, audience);
}
