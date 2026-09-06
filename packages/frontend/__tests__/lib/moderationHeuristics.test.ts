import { describe, expect, it } from "vitest";

import {
  SLOP_MODEL_REGEX,
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
  const totalTokens = overrides.totalTokens ?? 1_200_000;
  const slopModels = overrides.slopModels ?? [];
  return {
    userId: "user-1",
    username: "normal",
    avatarUrl: null,
    leaderboardHidden: false,
    totalTokens,
    totalCost: 12,
    submitCount: 4,
    hasBackfill: false,
    dailyTokens: 1_200_000,
    nearDuplicateCount: 0,
    slopModels,
    slopTokens:
      overrides.slopTokens !== undefined
        ? overrides.slopTokens
        : slopModels.length > 0
        ? totalTokens
        : 0,
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

  it("flags invented model names and quotes them verbatim", () => {
    // The real three variants on the rank-1 account.
    const scored = scoreCandidate(
      row({
        slopModels: ["slopllm-5m", "slopai/slopllm-5m", "slopai/slopllm:5m"],
      }),
      CONTEXT
    );

    expect(signalKeys(scored)).toContain("slopModelName");
    // Quoted so the reviewer judges the string itself rather than trusting the
    // match — the name is the evidence.
    expect(scored.signals.find((s) => s.key === "slopModelName")?.label).toContain(
      '"slopllm-5m"'
    );
  });

  it("outweighs every other single signal, since a name cannot be innocent", () => {
    const slop = scoreCandidate(row({ slopModels: ["slopllm-5m"] }), CONTEXT);
    const duplicate = scoreCandidate(row({ nearDuplicateCount: 1 }), CONTEXT);

    expect(slop.score).toBeGreaterThan(duplicate.score);
  });

  it("summarises rather than listing every match", () => {
    const scored = scoreCandidate(
      row({ slopModels: ["a-slop", "b-slop", "c-slop", "d-slop", "e-slop"] }),
      CONTEXT
    );

    expect(scored.signals[0].label).toContain("and 2 more");
  });

  it("does not flag an account with no invented names", () => {
    expect(signalKeys(scoreCandidate(row({ slopModels: [] }), CONTEXT))).not.toContain(
      "slopModelName"
    );
  });

  it("scales slopModelName weight by the matching models' token share", () => {
    // Real separation from issue #1265:
    // Account A: 9.007e15 slop tokens / 9.026e15 account tokens (~99.8% share) -> weight 34.9
    const accountA = scoreCandidate(
      row({
        username: "account-a",
        totalTokens: 9_026_000_000_000_000,
        slopModels: ["slopai/slopllm-5m"],
        slopTokens: 9_007_000_000_000_000,
      }),
      CONTEXT
    );
    const slopSignal = accountA.signals.find((s) => s.key === "slopModelName");
    expect(slopSignal).toBeDefined();
    expect(slopSignal!.weight).toBeCloseTo(34.9, 1);
  });

  it("drops slopModelName signal when the weight rounds to zero", () => {
    // 2 slop tokens on an account with normal usage -> weight rounds to 0 -> signal dropped (#1265)
    const accountB = scoreCandidate(
      row({
        username: "account-b",
        slopModels: ["fake-test-model"],
        slopTokens: 2,
      }),
      CONTEXT
    );
    expect(signalKeys(accountB)).not.toContain("slopModelName");
    expect(accountB.signals).toEqual([]);
    expect(accountB.score).toBe(0);

    // 0 slop tokens (mock provider in config) -> weight 0 -> signal dropped (#1265)
    const accountC = scoreCandidate(
      row({
        username: "account-c",
        slopModels: ["fake-api"],
        slopTokens: 0,
      }),
      CONTEXT
    );
    expect(signalKeys(accountC)).not.toContain("slopModelName");
    expect(accountC.signals).toEqual([]);
    expect(accountC.score).toBe(0);
  });

  it("drops slopModelName when breakdown is present but totalTokens is zero", () => {
    const zeroTokens = scoreCandidate(
      row({
        username: "zero-token-account",
        totalTokens: 0,
        slopModels: ["fake-api"],
        slopTokens: 0,
      }),
      CONTEXT
    );
    expect(signalKeys(zeroTokens)).not.toContain("slopModelName");
    expect(zeroTokens.signals).toEqual([]);
    expect(zeroTokens.score).toBe(0);
  });

  it("retains full slopModelName weight when breakdown data is unavailable (null slopTokens)", () => {
    // Legacy submissions or submissions without daily breakdown data cannot compute
    // token share, so they retain the original full weight of 35.
    const legacy = scoreCandidate(
      row({
        username: "legacy-user",
        slopModels: ["slopai/slopllm-5m"],
        slopTokens: null,
      }),
      CONTEXT
    );
    const slopSignal = legacy.signals.find((s) => s.key === "slopModelName");
    expect(slopSignal).toBeDefined();
    expect(slopSignal!.weight).toBe(35);
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

  it("flags an implied per-token price above every provider's list price", () => {
    const tooExpensive = scoreCandidate(
      row({ totalTokens: 1_000, totalCost: 500 }),
      CONTEXT
    );

    expect(signalKeys(tooExpensive)).toContain("impliedRate");
  });

  it("does not flag a very low implied rate, which local and free models produce", () => {
    // Measured against production: a 1e-7 floor flagged 38 ordinary accounts
    // against 3 genuine ones. Ollama and LM Studio cost nothing, free tiers
    // cost nothing, and cache reads are far cheaper than input tokens, so a
    // low blended rate is normal heavy usage rather than evidence of anything.
    // @adheizal's real figures, with the real median (~5.8e9, derived from
    // grenadeoftacoss reporting 1,550,270x it) and daily rows that agree with
    // the stored total, so this isolates the implied-rate signal alone.
    const localModels = scoreCandidate(
      row({
        totalTokens: 87_931_302_128,
        totalCost: 6_232,
        dailyTokens: 87_931_302_128,
      }),
      { siteTokens: 9_078_292_663_926_388, medianTokens: 5_822_000_000 }
    );
    const nearlyFree = scoreCandidate(
      row({ totalTokens: 1_000_000_000, totalCost: 1, dailyTokens: 1_000_000_000 }),
      { siteTokens: 9_078_292_663_926_388, medianTokens: 5_822_000_000 }
    );

    expect(signalKeys(localModels)).not.toContain("impliedRate");
    expect(signalKeys(nearlyFree)).not.toContain("impliedRate");
    // Drops out of the queue entirely rather than sitting there as permanent
    // noise — which is what the old floor did to 38 accounts like this one.
    expect(localModels.signals).toEqual([]);
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

  it("omits false-positive accounts with zero or negligible slop tokens from the ranked queue", () => {
    const accountA = row({
      userId: "a",
      username: "account-a",
      totalTokens: 9_026_000_000_000_000,
      slopModels: ["slopai/slopllm-5m"],
      slopTokens: 9_007_000_000_000_000,
    });
    const accountB = row({
      userId: "b",
      username: "account-b",
      slopModels: ["fake-test-model"],
      slopTokens: 2,
    });
    const accountC = row({
      userId: "c",
      username: "account-c",
      slopModels: ["fake-api"],
      slopTokens: 0,
    });

    const ranked = rankCandidates([accountA, accountB, accountC], CONTEXT);

    // Only account-a has enough weight to remain in the queue; account-b and account-c drop out (#1265)
    expect(ranked.map((c) => c.username)).toEqual(["account-a"]);
  });
});

describe("SLOP_MODEL_REGEX", () => {
  // The pattern is interpolated into a Postgres `~*` comparison, so this is a
  // JS approximation of that operator. Both are POSIX-flavoured and the
  // constructs used here -- alternation, a negated class, an anchor -- behave
  // identically, which is enough to pin the boundary this test is about.
  const matches = (name: string) => new RegExp(SLOP_MODEL_REGEX, "i").test(name);

  it("matches an invented name with no delimiter before the marker", () => {
    // The case the pattern list exists for. Requiring delimiters on BOTH sides
    // would silently stop catching it, which is why only the left is anchored.
    expect(matches("slopllm")).toBe(true);
    expect(matches("SlopLLM")).toBe(true);
  });

  it("matches the marker at a segment boundary", () => {
    expect(matches("slop-llm")).toBe(true);
    expect(matches("slopai/slopllm:5m")).toBe(true);
    expect(matches("gpt-4-fake")).toBe(true);
    expect(matches("my_dummy_model")).toBe(true);
  });

  it("does not match a marker buried inside a longer word", () => {
    // The false-positive class: a future legitimate id that merely contains
    // one of these words should not enter the queue.
    expect(matches("notaslopname")).toBe(false);
    expect(matches("xfakey")).toBe(false);
  });

  it("leaves ordinary model ids alone", () => {
    expect(matches("claude-sonnet-4-5")).toBe(false);
    expect(matches("deepseek-v3")).toBe(false);
    expect(matches("gpt-5.4")).toBe(false);
  });
});
