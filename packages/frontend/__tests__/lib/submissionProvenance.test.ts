import { describe, expect, it } from "vitest";
import {
  generateSubmissionHash,
  validateSubmission,
  type SubmissionData,
} from "@/lib/validation/submission";

// A minimal, internally-consistent submission (one day, one client) that
// passes Level 1 validation. `provenance` is layered on per test.
function buildSubmission(provenance?: unknown): Record<string, unknown> {
  const tokens = {
    input: 100,
    output: 100,
    cacheRead: 100,
    cacheWrite: 0,
    reasoning: 0,
  };
  const base: Record<string, unknown> = {
    meta: {
      generatedAt: "2026-07-14T00:00:00.000Z",
      version: "4.5.3",
      dateRange: { start: "2026-05-11", end: "2026-05-11" },
    },
    summary: {
      totalTokens: 300,
      totalCost: 1.5,
      totalDays: 1,
      activeDays: 1,
      averagePerDay: 1.5,
      maxCostInSingleDay: 1.5,
      clients: ["codex"],
      models: ["gpt-5.5"],
    },
    years: [
      {
        year: "2026",
        totalTokens: 300,
        totalCost: 1.5,
        range: { start: "2026-05-11", end: "2026-05-11" },
      },
    ],
    contributions: [
      {
        date: "2026-05-11",
        totals: { tokens: 300, cost: 1.5, messages: 0 },
        intensity: 4,
        tokenBreakdown: tokens,
        clients: [{ client: "codex", modelId: "gpt-5.5", tokens, cost: 1.5, messages: 0 }],
      },
    ],
  };
  if (provenance !== undefined) {
    base.provenance = provenance;
  }
  return base;
}

describe("submission provenance", () => {
  it("accepts a submission with no provenance (backward compatible)", () => {
    const result = validateSubmission(buildSubmission());
    expect(result.errors).toEqual([]);
    expect(result.valid).toBe(true);
    expect(result.data?.provenance).toBeUndefined();
  });

  it("accepts and carries a backfill provenance tag", () => {
    const result = validateSubmission(
      buildSubmission({ origin: "backfill", importer: "clawdboard" })
    );
    expect(result.valid).toBe(true);
    expect(result.data?.provenance).toEqual({
      origin: "backfill",
      importer: "clawdboard",
    });
  });

  it("accepts origin 'cli' without an importer", () => {
    const result = validateSubmission(buildSubmission({ origin: "cli" }));
    expect(result.valid).toBe(true);
    expect(result.data?.provenance).toEqual({ origin: "cli" });
  });

  it("rejects an unknown provenance origin", () => {
    const result = validateSubmission(buildSubmission({ origin: "totally-made-up" }));
    expect(result.valid).toBe(false);
  });

  it("excludes provenance from the idempotency hash", () => {
    // Backfilled data must not change the content hash: the same usage should
    // dedupe against a prior live submission regardless of the tag.
    const without = validateSubmission(buildSubmission());
    const backfill = validateSubmission(
      buildSubmission({ origin: "backfill", importer: "clawdboard" })
    );
    const cli = validateSubmission(buildSubmission({ origin: "cli" }));
    expect(Boolean(without.data && backfill.data && cli.data)).toBe(true);

    const baseHash = generateSubmissionHash(without.data as SubmissionData);
    expect(generateSubmissionHash(backfill.data as SubmissionData)).toBe(baseHash);
    expect(generateSubmissionHash(cli.data as SubmissionData)).toBe(baseHash);
  });
});
