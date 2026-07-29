import { describe, expect, it } from "vitest";

import {
  rankCandidates,
  scoreCandidate,
  type CandidateContext,
  type CandidateRow,
} from "@/lib/moderation/heuristics";

/**
 * The real site totals at the time this was written.
 *
 * These exceed Number.MAX_SAFE_INTEGER, so they are approximate as JS numbers
 * — which is true of the production values too, since submissions.total_tokens
 * is read in `number` mode. Harmless here: every signal is a ratio, and a few
 * units of drift at 10^15 cannot move a threshold.
 */
const CONTEXT: CandidateContext = {
  siteTokens: 9_078_199_482_735_296,
  medianTokens: 1_000_000,
};

function row(overrides: Partial<CandidateRow> = {}): CandidateRow {
  return {
    userId: "user-1",
    username: "normal",
    avatarUrl: null,
    leaderboardHidden: false,
    totalTokens: 1_200_000,
    totalCost: 12,
    submitCount: 4,
    hasBackfill: false,
    dailyTokens: 1_200_000,
    nearDuplicateCount: 0,
    ...overrides,
  };
}

function signalKeys(candidate: { signals: { key: string }[] }): string[] {
  return candidate.signals.map((signal) => signal.key);
}

describe("scoreCandidate", () => {
  it("flags nothing for an ordinary user", () => {
    const scored = scoreCandidate(row(), CONTEXT);

    expect(scored.signals).toEqual([]);
    expect(scored.score).toBe(0);
  });

  it("flags an account holding most of the site's tokens", () => {
    // The real rank-1 account: 99.4% of every token on the site.
    const scored = scoreCandidate(
      row({ username: "grenadeoftacoss", totalTokens: 9_025_906_844_219_236 }),
      CONTEXT
    );

    expect(signalKeys(scored)).toContain("siteShare");
    expect(scored.signals.find((s) => s.key === "siteShare")?.label).toContain("99.4%");
  });

  it("flags a token total that matches another account almost exactly", () => {
    // Ranks 2 and 3 differed by exactly one token — one dataset, two accounts.
    const scored = scoreCandidate(row({ nearDuplicateCount: 1 }), CONTEXT);

    expect(signalKeys(scored)).toContain("duplicateTotal");
    expect(scored.signals.find((s) => s.key === "duplicateTotal")?.label).toContain(
      "matches another account"
    );
  });

  it("attributes a daily-sum mismatch to our own inflation bug, not the user", () => {
    const scored = scoreCandidate(
      row({ totalTokens: 10_000_000, dailyTokens: 1_000_000 }),
      CONTEXT
    );

    const label = scored.signals.find((s) => s.key === "dailyMismatch")?.label;
    expect(label).toContain("#960");
    expect(label).toContain("not necessarily the user");
  });

  it("does not flag a daily mismatch when there are no daily rows at all", () => {
    // An older submission shape, not evidence of anything.
    const scored = scoreCandidate(row({ dailyTokens: 0 }), CONTEXT);

    expect(signalKeys(scored)).not.toContain("dailyMismatch");
  });

  it("flags an implied per-token price outside provider pricing", () => {
    const tooExpensive = scoreCandidate(
      row({ totalTokens: 1_000, totalCost: 500 }),
      CONTEXT
    );
    const tooCheap = scoreCandidate(
      row({ totalTokens: 1_000_000_000_000, totalCost: 1 }),
      CONTEXT
    );

    expect(signalKeys(tooExpensive)).toContain("impliedRate");
    expect(signalKeys(tooCheap)).toContain("impliedRate");
  });

  it("never divides by zero on an empty site or a zero-token user", () => {
    const emptySite = scoreCandidate(row(), { siteTokens: 0, medianTokens: 0 });
    const zeroUser = scoreCandidate(row({ totalTokens: 0, dailyTokens: 0 }), CONTEXT);

    expect(Number.isFinite(emptySite.score)).toBe(true);
    expect(Number.isFinite(zeroUser.score)).toBe(true);
  });
});

describe("rankCandidates", () => {
  it("orders the worst offender first", () => {
    const ranked = rankCandidates(
      [
        row({ userId: "u1", username: "clean" }),
        row({
          userId: "u2",
          username: "worst",
          totalTokens: 9_025_906_844_219_236,
          nearDuplicateCount: 1,
        }),
        row({ userId: "u3", username: "middling", nearDuplicateCount: 1 }),
      ],
      CONTEXT
    );

    expect(ranked.map((c) => c.username)).toEqual(["worst", "middling"]);
  });

  it("keeps already-hidden users in the queue so decisions stay reversible", () => {
    // Nothing suspicious about them any more, but they must remain visible or
    // a past hide becomes impossible to find and undo.
    const ranked = rankCandidates(
      [row({ userId: "u1", username: "previously-hidden", leaderboardHidden: true })],
      CONTEXT
    );

    expect(ranked).toHaveLength(1);
    expect(ranked[0].leaderboardHidden).toBe(true);
    expect(ranked[0].signals).toEqual([]);
  });

  it("omits users with no signals and no prior decision", () => {
    const ranked = rankCandidates([row({ username: "clean" })], CONTEXT);

    expect(ranked).toEqual([]);
  });

  it("breaks score ties by username so the queue order is stable", () => {
    const ranked = rankCandidates(
      [
        row({ userId: "u1", username: "zoe", nearDuplicateCount: 1 }),
        row({ userId: "u2", username: "adam", nearDuplicateCount: 1 }),
      ],
      CONTEXT
    );

    expect(ranked.map((c) => c.username)).toEqual(["adam", "zoe"]);
  });
});
