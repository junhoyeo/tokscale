import { describe, expect, it } from "vitest";

import {
  mergeClientBreakdowns,
  type ClientBreakdownData,
} from "../../src/lib/db/helpers";

function makeClientBreakdown(tokens: number, modelId = "claude-sonnet-4"): ClientBreakdownData {
  return {
    tokens,
    cost: tokens / 100,
    input: Math.floor(tokens * 0.6),
    output: Math.floor(tokens * 0.3),
    cacheRead: Math.floor(tokens * 0.05),
    cacheWrite: Math.floor(tokens * 0.05),
    reasoning: 0,
    messages: 1,
    modelId,
    models: {
      [modelId]: {
        tokens,
        cost: tokens / 100,
        input: Math.floor(tokens * 0.6),
        output: Math.floor(tokens * 0.3),
        cacheRead: Math.floor(tokens * 0.05),
        cacheWrite: Math.floor(tokens * 0.05),
        reasoning: 0,
        messages: 1,
      },
    },
  };
}

describe("mergeClientBreakdowns", () => {
  it("preserves same-client data from different machines", () => {
    const first = mergeClientBreakdowns(
      {},
      { claude: makeClientBreakdown(1000) },
      new Set(["claude"]),
      "machine-a",
      "CLI on a"
    );

    const second = mergeClientBreakdowns(
      first,
      { claude: makeClientBreakdown(500) },
      new Set(["claude"]),
      "machine-b",
      "CLI on b"
    );

    expect(second.claude.tokens).toBe(1500);
    expect(second.claude.instances?.["machine-a"]?.tokens).toBe(1000);
    expect(second.claude.instances?.["machine-b"]?.tokens).toBe(500);
  });

  it("replaces only the matching machine on resubmit", () => {
    const existing = {
      claude: {
        ...makeClientBreakdown(1500),
        instances: {
          "machine-a": { ...makeClientBreakdown(1000), sourceName: "CLI on a" },
          "machine-b": { ...makeClientBreakdown(500), sourceName: "CLI on b" },
        },
      },
    };

    const merged = mergeClientBreakdowns(
      existing,
      { claude: makeClientBreakdown(700) },
      new Set(["claude"]),
      "machine-b",
      "CLI on b"
    );

    expect(merged.claude.tokens).toBe(1700);
    expect(merged.claude.instances?.["machine-a"]?.tokens).toBe(1000);
    expect(merged.claude.instances?.["machine-b"]?.tokens).toBe(700);
  });

  it("assigns legacy rows to the first identified source on upgrade", () => {
    const merged = mergeClientBreakdowns(
      { claude: makeClientBreakdown(1000) },
      { claude: makeClientBreakdown(800) },
      new Set(["claude"]),
      "machine-a",
      "CLI on a"
    );

    expect(merged.claude.tokens).toBe(800);
    expect(Object.keys(merged.claude.instances || {})).toEqual(["machine-a"]);
  });
});
