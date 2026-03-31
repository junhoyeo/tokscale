import { describe, expect, it } from "vitest";
import { validateSubmission } from "../../src/lib/validation/submission";

function createValidSubmission(metaOverrides: Partial<Record<"sourceId" | "sourceName", string>> = {}) {
  return {
    meta: {
      generatedAt: "2026-03-31T00:00:00.000Z",
      version: "2.0.14",
      dateRange: {
        start: "2026-03-01",
        end: "2026-03-01",
      },
      ...metaOverrides,
    },
    summary: {
      totalTokens: 150,
      totalCost: 1.5,
      totalDays: 1,
      activeDays: 1,
      averagePerDay: 1.5,
      maxCostInSingleDay: 1.5,
      clients: ["claude"],
      models: ["claude-sonnet-4-20250514"],
    },
    years: [
      {
        year: "2026",
        totalTokens: 150,
        totalCost: 1.5,
        range: {
          start: "2026-03-01",
          end: "2026-03-01",
        },
      },
    ],
    contributions: [
      {
        date: "2026-03-01",
        timestampMs: 1740787200000,
        totals: {
          tokens: 150,
          cost: 1.5,
          messages: 2,
        },
        intensity: 1,
        tokenBreakdown: {
          input: 100,
          output: 50,
          cacheRead: 0,
          cacheWrite: 0,
          reasoning: 0,
        },
        clients: [
          {
            client: "claude",
            modelId: "claude-sonnet-4-20250514",
            tokens: {
              input: 100,
              output: 50,
              cacheRead: 0,
              cacheWrite: 0,
              reasoning: 0,
            },
            cost: 1.5,
            messages: 2,
          },
        ],
      },
    ],
  };
}

describe("validateSubmission", () => {
  it("treats blank optional source metadata as absent", () => {
    const result = validateSubmission(
      createValidSubmission({
        sourceId: "   ",
        sourceName: "\t",
      })
    );

    expect(result.valid).toBe(true);
    expect(result.data?.meta.sourceId).toBeUndefined();
    expect(result.data?.meta.sourceName).toBeUndefined();
  });
});
