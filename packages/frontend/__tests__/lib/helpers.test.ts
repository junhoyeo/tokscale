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
});
