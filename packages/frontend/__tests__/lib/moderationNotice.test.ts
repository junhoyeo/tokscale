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
