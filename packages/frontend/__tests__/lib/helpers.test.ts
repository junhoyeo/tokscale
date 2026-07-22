import { describe, expect, it } from "vitest";
import { mergeClientBreakdownsWithRegressionGuard } from "../../src/lib/db/helpers";

// Minimal client breakdown fixture
function makeClient(tokens: number, messages: number, modelCount: number) {
  const models: Record<string, { tokens: number; cost: number; input: number; output: number; cacheRead: number; cacheWrite: number; reasoning: number; messages: number }> = {};
  for (let i = 0; i < modelCount; i++) {
    models[`model-${i}`] = { tokens, cost: 0, input: tokens, output: 0, cacheRead: 0, cacheWrite: 0, reasoning: 0, messages };
  }
  return {
    tokens,
    cost: 0,
    input: tokens,
    output: 0,
    cacheRead: 0,
    cacheWrite: 0,
    reasoning: 0,
    messages,
    models,
  };
}

describe("mergeClientBreakdownsWithRegressionGuard", () => {
  it("preserves existing when incoming has fewer tokens and equal coverage (A2 regression guard)", () => {
    // Before the A2 fix, equal coverage + fewer tokens would NOT be preserved
    // because the guard required BOTH fewer tokens AND lower coverage.
    const existing = { codex: makeClient(1000, 5, 2) };
    // Same message count and model count, but fewer tokens — signals a parse regression
    const incoming = { codex: makeClient(800, 5, 2) };

    const result = mergeClientBreakdownsWithRegressionGuard(
      existing,
      incoming,
      new Set(["codex"])
    );

    expect(result.merged.codex.tokens).toBe(1000);
    expect(result.warnings).toHaveLength(1);
    expect(result.warnings[0]).toContain("1,000");
    expect(result.warnings[0]).toContain("800");
  });

  it("preserves existing when incoming has fewer tokens and lower coverage", () => {
    const existing = { codex: makeClient(1000, 5, 2) };
    const incoming = { codex: makeClient(800, 3, 1) };

    const result = mergeClientBreakdownsWithRegressionGuard(
      existing,
      incoming,
      new Set(["codex"])
    );

    expect(result.merged.codex.tokens).toBe(1000);
    expect(result.warnings).toHaveLength(1);
  });

  it("accepts incoming when it has more tokens than existing", () => {
    const existing = { codex: makeClient(800, 5, 2) };
    const incoming = { codex: makeClient(1000, 5, 2) };

    const result = mergeClientBreakdownsWithRegressionGuard(
      existing,
      incoming,
      new Set(["codex"])
    );

    expect(result.merged.codex.tokens).toBe(1000);
    expect(result.warnings).toHaveLength(0);
  });

  it("accepts incoming when tokens are equal", () => {
    const existing = { codex: makeClient(1000, 5, 2) };
    const incoming = { codex: makeClient(1000, 5, 2) };

    const result = mergeClientBreakdownsWithRegressionGuard(
      existing,
      incoming,
      new Set(["codex"])
    );

    expect(result.merged.codex.tokens).toBe(1000);
    expect(result.warnings).toHaveLength(0);
  });

  it("preserves existing client that disappeared from incoming resubmit", () => {
    const existing = { codex: makeClient(1000, 5, 2), cursor: makeClient(500, 3, 1) };
    const incoming = { codex: makeClient(1200, 6, 2) };

    const result = mergeClientBreakdownsWithRegressionGuard(
      existing,
      incoming,
      new Set(["codex", "cursor"])
    );

    // codex is updated (more tokens)
    expect(result.merged.codex.tokens).toBe(1200);
    // cursor is preserved (disappeared from incoming but had tokens)
    expect(result.merged.cursor.tokens).toBe(500);
    expect(result.warnings).toHaveLength(1);
    expect(result.warnings[0]).toContain("cursor");
  });

  describe("foldedClientFloors parameter (alias-fold double-count healing)", () => {
    it("lets a lower incoming value at or above the floor replace a folded client", () => {
      // Simulates the healed state normalizeClientBreakdownAliases would produce:
      // a stored kilocode(100)+kilo(100) double count folded into a single
      // 200-token "kilo" entry with a floor of 100 (largest single component),
      // and the incoming complete-day resubmit reporting the true 100-token
      // total — exactly at the floor, so it heals.
      const existing = { kilo: makeClient(200, 5, 1) };
      const incoming = { kilo: makeClient(100, 5, 1) };

      const result = mergeClientBreakdownsWithRegressionGuard(
        existing,
        incoming,
        new Set(["kilo"]),
        new Map([["kilo", 100]])
      );

      expect(result.merged.kilo.tokens).toBe(100);
      expect(result.warnings).toHaveLength(1);
      expect(result.warnings[0]).toContain("Healed");
      expect(result.warnings[0]).not.toMatch(/parser regression|Preserved/i);
      // Healed -> the fold is resolved; nothing to preserve raw keys for.
      expect(result.foldPreservedClients.size).toBe(0);
    });

    it("keeps the regression guard when the incoming value is below the floor", () => {
      // An incoming value below the largest single contribution to the fold
      // is indistinguishable from a partial re-parse — the exact data-loss
      // case the guard exists for — so the fold must be preserved, not
      // "healed" down to the partial value.
      const existing = { kilo: makeClient(200, 5, 1) };
      const incoming = { kilo: makeClient(40, 5, 1) };

      const result = mergeClientBreakdownsWithRegressionGuard(
        existing,
        incoming,
        new Set(["kilo"]),
        new Map([["kilo", 100]])
      );

      expect(result.merged.kilo.tokens).toBe(200);
      expect(result.warnings).toHaveLength(1);
      expect(result.warnings[0]).toContain("Preserved");
      // Preserved fold -> the caller must write the raw alias keys back so
      // the heal floor survives this partial resubmit.
      expect(result.foldPreservedClients).toEqual(new Set(["kilo"]));
    });

    it("keeps the normal regression guard for a non-folded client with the same shape", () => {
      // Same numbers as above, but the client was NOT flagged as folded
      // (e.g. a plain rename-only case) -- normal preserve-and-warn behavior
      // must still apply.
      const existing = { kilo: makeClient(200, 5, 1) };
      const incoming = { kilo: makeClient(100, 5, 1) };

      const result = mergeClientBreakdownsWithRegressionGuard(
        existing,
        incoming,
        new Set(["kilo"])
      );

      expect(result.merged.kilo.tokens).toBe(200);
      expect(result.warnings).toHaveLength(1);
      expect(result.warnings[0]).toContain("Preserved");
      // Not folded -> nothing to preserve raw keys for.
      expect(result.foldPreservedClients.size).toBe(0);
    });

    it("marks an untouched folded client as preserved (carry-over must not collapse it)", () => {
      // The incoming submission doesn't claim kilo at all (different client
      // set); the folded entry is carried over by the spread and must still
      // be flagged so the raw keys survive the writeback.
      const existing = { kilo: makeClient(200, 5, 1) };
      const incoming = { cursor: makeClient(50, 2, 1) };

      const result = mergeClientBreakdownsWithRegressionGuard(
        existing,
        incoming,
        new Set(["cursor"]),
        new Map([["kilo", 100]])
      );

      expect(result.merged.kilo.tokens).toBe(200);
      expect(result.merged.cursor.tokens).toBe(50);
      expect(result.foldPreservedClients).toEqual(new Set(["kilo"]));
    });

    it("keeps the folded value when the incoming submission does not include the client", () => {
      const existing = { kilo: makeClient(200, 5, 1) };

      const result = mergeClientBreakdownsWithRegressionGuard(
        existing,
        {},
        new Set(["kilo"]),
        new Map([["kilo", 100]])
      );

      expect(result.merged.kilo.tokens).toBe(200);
      expect(result.warnings).toHaveLength(1);
      expect(result.warnings[0]).toContain("disappeared");
      expect(result.foldPreservedClients).toEqual(new Set(["kilo"]));
    });
  });
});
