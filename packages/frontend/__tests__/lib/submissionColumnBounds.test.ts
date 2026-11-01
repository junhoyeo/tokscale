import { describe, expect, it } from "vitest";
import { validateSubmission } from "@/lib/validation/submission";

const baseTokens = {
  input: 100,
  output: 100,
  cacheRead: 100,
  cacheWrite: 0,
  reasoning: 0,
};

function buildValidSubmission(overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    meta: {
      generatedAt: "2026-05-01T00:00:00.000Z",
      version: "1.2.3",
      dateRange: { start: "2026-04-30", end: "2026-04-30" },
    },
    summary: {
      totalTokens: 300,
      totalCost: 1.5,
      totalDays: 1,
      activeDays: 1,
      averagePerDay: 1.5,
      maxCostInSingleDay: 1.5,
      clients: ["opencode"],
      models: ["claude-sonnet-4"],
    },
    years: [
      {
        year: "2026",
        totalTokens: 300,
        totalCost: 1.5,
        range: { start: "2026-04-30", end: "2026-04-30" },
      },
    ],
    contributions: [
      {
        date: "2026-04-30",
        activeTimeMs: 60_000,
        totals: { tokens: 300, cost: 1.5, messages: 1 },
        intensity: 1,
        tokenBreakdown: baseTokens,
        clients: [
          {
            client: "opencode",
            modelId: "claude-sonnet-4",
            tokens: baseTokens,
            cost: 1.5,
            messages: 1,
          },
        ],
      },
    ],
    timeMetrics: {
      totalActiveTimeMs: 60_000,
      longestContinuousMs: 60_000,
      maxConcurrentSessions: 2,
      sessionCount: 5,
    },
    ...overrides,
  };
}

describe("validateSubmission PostgreSQL column boundary guards", () => {
  it("accepts boundary values within PostgreSQL column limits", () => {
    const payload = buildValidSubmission({
      meta: {
        generatedAt: "2026-05-01T00:00:00.000Z",
        version: "12345678901234567890", // exactly 20 chars (varchar(20))
        dateRange: { start: "2026-04-30", end: "2026-04-30" },
      },
      timeMetrics: {
        totalActiveTimeMs: Number.MAX_SAFE_INTEGER,
        longestContinuousMs: Number.MAX_SAFE_INTEGER,
        maxConcurrentSessions: 2_147_483_647,
        sessionCount: 2_147_483_647,
      },
    });

    const result = validateSubmission(payload);
    expect(result.valid).toBe(true);
    expect(result.errors).toEqual([]);
  });

  it("rejects meta.version longer than 20 characters (submissions.cli_version varchar(20))", () => {
    const payload = buildValidSubmission({
      meta: {
        generatedAt: "2026-05-01T00:00:00.000Z",
        version: "x".repeat(21),
        dateRange: { start: "2026-04-30", end: "2026-04-30" },
      },
    });

    const result = validateSubmission(payload);
    expect(result.valid).toBe(false);
    expect(result.errors).toEqual(
      expect.arrayContaining([expect.stringContaining("meta.version")]),
    );
  });

  it("rejects daily or client cost >= 10^10 that overflows DECIMAL(14, 4)", () => {
    const overflowCost = 10_000_000_000;
    const payload = buildValidSubmission({
      summary: {
        totalTokens: 300,
        totalCost: overflowCost,
        totalDays: 1,
        activeDays: 1,
        averagePerDay: overflowCost,
        maxCostInSingleDay: overflowCost,
        clients: ["opencode"],
        models: ["claude-sonnet-4"],
      },
      years: [
        {
          year: "2026",
          totalTokens: 300,
          totalCost: overflowCost,
          range: { start: "2026-04-30", end: "2026-04-30" },
        },
      ],
      contributions: [
        {
          date: "2026-04-30",
          totals: { tokens: 300, cost: overflowCost, messages: 1 },
          intensity: 1,
          tokenBreakdown: baseTokens,
          clients: [
            {
              client: "opencode",
              modelId: "claude-sonnet-4",
              tokens: baseTokens,
              cost: overflowCost,
              messages: 1,
            },
          ],
        },
      ],
    });

    const result = validateSubmission(payload);
    expect(result.valid).toBe(false);
    expect(result.errors).toEqual(
      expect.arrayContaining([
        expect.stringContaining("contributions.0.totals.cost"),
        expect.stringContaining("contributions.0.clients.0.cost"),
      ]),
    );
  });

  it("rejects multi-client day cost sum that exceeds DECIMAL(14, 4) via 1% tolerance", () => {
    const dayCost = 9_999_999_999;
    const perClientCost = 5_025_000_000; // sum = 10,050,000,000 (within 1% of dayCost, but > 9,999,999,999.9999)
    const halfTokens = {
      input: 50,
      output: 50,
      cacheRead: 50,
      cacheWrite: 0,
      reasoning: 0,
    };
    const payload = buildValidSubmission({
      summary: {
        totalTokens: 300,
        totalCost: dayCost,
        totalDays: 1,
        activeDays: 1,
        averagePerDay: dayCost,
        maxCostInSingleDay: dayCost,
        clients: ["opencode", "cursor"],
        models: ["claude-sonnet-4"],
      },
      years: [
        {
          year: "2026",
          totalTokens: 300,
          totalCost: dayCost,
          range: { start: "2026-04-30", end: "2026-04-30" },
        },
      ],
      contributions: [
        {
          date: "2026-04-30",
          totals: { tokens: 300, cost: dayCost, messages: 2 },
          intensity: 1,
          tokenBreakdown: baseTokens,
          clients: [
            {
              client: "opencode",
              modelId: "claude-sonnet-4",
              tokens: halfTokens,
              cost: perClientCost,
              messages: 1,
            },
            {
              client: "cursor",
              modelId: "claude-sonnet-4",
              tokens: halfTokens,
              cost: perClientCost,
              messages: 1,
            },
          ],
        },
      ],
    });

    const result = validateSubmission(payload);
    expect(result.valid).toBe(false);
    expect(result.errors).toEqual(
      expect.arrayContaining([
        expect.stringContaining("exceeds maximum allowed daily cost"),
      ]),
    );
  });

  it("rejects timeMetrics.sessionCount and maxConcurrentSessions above PostgreSQL int4 max", () => {
    const payload = buildValidSubmission({
      timeMetrics: {
        totalActiveTimeMs: 60_000,
        longestContinuousMs: 60_000,
        maxConcurrentSessions: 2_147_483_648,
        sessionCount: 2_147_483_648,
      },
    });

    const result = validateSubmission(payload);
    expect(result.valid).toBe(false);
    expect(result.errors).toEqual(
      expect.arrayContaining([
        expect.stringContaining("timeMetrics.maxConcurrentSessions"),
        expect.stringContaining("timeMetrics.sessionCount"),
      ]),
    );
  });

  it("rejects activeTimeMs fields above Number.MAX_SAFE_INTEGER", () => {
    const unsafeInt = 9_007_199_254_740_992; // 2^53 (even, passes z.number().int() without .max())
    const payload = buildValidSubmission({
      contributions: [
        {
          date: "2026-04-30",
          activeTimeMs: unsafeInt,
          totals: { tokens: 300, cost: 1.5, messages: 1 },
          intensity: 1,
          tokenBreakdown: baseTokens,
          clients: [
            {
              client: "opencode",
              modelId: "claude-sonnet-4",
              tokens: baseTokens,
              cost: 1.5,
              messages: 1,
            },
          ],
        },
      ],
      timeMetrics: {
        totalActiveTimeMs: unsafeInt,
        longestContinuousMs: unsafeInt,
        maxConcurrentSessions: 1,
        sessionCount: 1,
      },
    });

    const result = validateSubmission(payload);
    expect(result.valid).toBe(false);
    expect(result.errors).toEqual(
      expect.arrayContaining([
        expect.stringContaining("contributions.0.activeTimeMs"),
        expect.stringContaining("timeMetrics.totalActiveTimeMs"),
        expect.stringContaining("timeMetrics.longestContinuousMs"),
      ]),
    );
  });

  it("rejects impossible calendar dates like 2025-02-29 or 2025-99-99", () => {
    const payload = buildValidSubmission({
      meta: {
        generatedAt: "2026-05-01T00:00:00.000Z",
        version: "1.2.3",
        dateRange: { start: "2025-02-29", end: "2025-99-99" },
      },
      contributions: [
        {
          date: "2025-02-29",
          totals: { tokens: 300, cost: 1.5, messages: 1 },
          intensity: 1,
          tokenBreakdown: baseTokens,
          clients: [
            {
              client: "opencode",
              modelId: "claude-sonnet-4",
              tokens: baseTokens,
              cost: 1.5,
              messages: 1,
            },
          ],
        },
      ],
    });

    const result = validateSubmission(payload);
    expect(result.valid).toBe(false);
    expect(result.errors).toEqual(
      expect.arrayContaining([
        expect.stringContaining("meta.dateRange.start: Invalid calendar date"),
        expect.stringContaining("meta.dateRange.end: Invalid calendar date"),
        expect.stringContaining("contributions.0.date: Invalid calendar date"),
      ]),
    );
  });
});
