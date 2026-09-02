import { describe, expect, it } from "vitest";

import { moderationNoticeFor } from "@/lib/moderation/notice";

const CONTACT = "i@junho.io";

describe("moderationNoticeFor", () => {
  it("always gives the owner somewhere to appeal", () => {
    for (const reason of ["Abuse", "Under investigation", "Data issue on our side", null]) {
      expect(moderationNoticeFor(reason).message).toContain(CONTACT);
    }
  });

  it("tells someone hidden for abuse exactly that", () => {
    const notice = moderationNoticeFor("Abuse");

    expect(notice.tone).toBe("enforcement");
    expect(notice.message).toContain("abusing");
  });

  it("never blames the user when the cause is our own data problem", () => {
    // Hiding someone because of ratchet inflation (#960) and then telling them
    // they abused the leaderboard is a false accusation they cannot appeal.
    const notice = moderationNoticeFor("Data issue on our side");

    expect(notice.tone).toBe("our-fault");
    expect(notice.message).toContain("not anything you did");
    expect(notice.message).not.toMatch(/abus/i);
  });

  it("does not accuse anyone when the reason is unknown or missing", () => {
    // A flag set without an audit row means we do not know what happened, and
    // "you abused this" is the one message that must never be sent on a guess.
    for (const reason of [null, "Some reason added later"]) {
      const notice = moderationNoticeFor(reason);

      expect(notice.tone).toBe("pending");
      expect(notice.message).not.toMatch(/abus/i);
      expect(notice.message).toContain("rank badges show N/A");
    }
  });

  it("reveals nothing about how the account was identified", () => {
    // Same rule as the stored reasons: a notice that names the signal is
    // evasion instructions.
    for (const reason of ["Abuse", "Under investigation", "Data issue on our side", null]) {
      const message = moderationNoticeFor(reason).message.toLowerCase();

      for (const leak of ["model name", "duplicate", "median", "daily", "per-token", "slop"]) {
        expect(message).not.toContain(leak);
      }
    }
  });
});

const REASONS = ["Abuse", "Under investigation", "Data issue on our side", null] as const;

describe("moderationNoticeFor, public audience", () => {
  it("never calls the account the reader's own", () => {
    // "Your profile and totals" shown to a visitor claims the account belongs
    // to whoever is reading. Every possessive has to move to third person.
    // "you" survives only in the appeal clause, which is addressed to whoever
    // spotted the mistake and is true for any reader.
    for (const reason of REASONS) {
      const { title, message } = moderationNoticeFor(reason, "public");

      for (const text of [title, message]) {
        expect(text).not.toMatch(/\byours?\b/i);
      }
    }
  });

  it("keeps the appeal address for whoever notices a wrong call", () => {
    for (const reason of REASONS) {
      expect(moderationNoticeFor(reason, "public").message).toContain(CONTACT);
    }
  });

  it("says everything the owner is told, not a shortened version", () => {
    // A visitor handed "this account was removed" and nothing else has a
    // verdict with no explanation of what it means. These are the clauses an
    // earlier draft cut; each states a consequence the bare outcome does not.
    const clauses: Array<[string, string]> = [
      ["Abuse", "no longer holds a ranking position"],
      ["Under investigation", "rank badges show N/A"],
      ["Data issue on our side", "will be restored once the underlying issue is corrected"],
    ];

    for (const [reason, clause] of clauses) {
      expect(moderationNoticeFor(reason, "public").message).toContain(clause);
    }
  });

  it("matches the owner wording clause for clause", () => {
    // The audience changes who the account is called, and nothing else. If the
    // two ever diverge in substance, one of them is telling someone a
    // different story about the same decision.
    const thirdToSecond = (text: string) =>
      text
        .replace(/This account's usage/g, "Your usage")
        .replace(/This account/g, "Your account")
        .replace(/Its profile/g, "Your profile")
        .replace(/it no longer holds/g, "you no longer hold")
        .replace(/while it is withheld/g, "while your account is withheld")
        .replace(/not anything the account owner did/g, "not anything you did");

    for (const reason of REASONS) {
      expect(thirdToSecond(moderationNoticeFor(reason, "public").message)).toBe(
        moderationNoticeFor(reason, "owner").message
      );
    }
  });

  it("classifies every reason the same way for both audiences", () => {
    // The audience decides the wording and nothing else. A reason that reads as
    // our fault in private must not read as enforcement in public.
    for (const reason of REASONS) {
      expect(moderationNoticeFor(reason, "public").tone).toBe(
        moderationNoticeFor(reason, "owner").tone
      );
    }
  });

  it("only says abuse in public when abuse is what was recorded", () => {
    expect(moderationNoticeFor("Abuse", "public").message).toContain("abusing");

    for (const reason of ["Under investigation", "Data issue on our side", null]) {
      expect(moderationNoticeFor(reason, "public").message).not.toMatch(/abus/i);
    }
  });

  it("clears the account owner in public when the cause is ours", () => {
    const notice = moderationNoticeFor("Data issue on our side", "public");

    expect(notice.tone).toBe("our-fault");
    expect(notice.message).toContain("not anything the account owner did");
  });

  it("reveals nothing about how the account was identified", () => {
    // Stricter in public than in private: naming the signal in front of
    // everyone is a published guide to evading it.
    for (const reason of REASONS) {
      const text = (
        moderationNoticeFor(reason, "public").title +
        " " +
        moderationNoticeFor(reason, "public").message
      ).toLowerCase();

      for (const leak of ["model name", "duplicate", "median", "daily", "per-token", "slop"]) {
        expect(text).not.toContain(leak);
      }
    }
  });

  it("defaults to the owner wording when no audience is given", () => {
    // The default is the narrower disclosure: a forgotten argument must not be
    // what publishes owner-addressed copy to strangers.
    for (const reason of REASONS) {
      expect(moderationNoticeFor(reason)).toEqual(moderationNoticeFor(reason, "owner"));
    }
  });
});

describe("moderationNoticeFor titles", () => {
  it("carries a title for every reason and audience", () => {
    for (const reason of REASONS) {
      for (const audience of ["owner", "public"] as const) {
        expect(moderationNoticeFor(reason, audience).title).not.toBe("");
      }
    }
  });

  it("moves the our-fault title to third person rather than dropping half of it", () => {
    expect(moderationNoticeFor("Data issue on our side", "owner").title).toContain("not yours");
    expect(moderationNoticeFor("Data issue on our side", "public").title).toContain("not theirs");
  });
});
