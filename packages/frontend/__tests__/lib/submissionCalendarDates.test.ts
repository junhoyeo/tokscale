import { describe, expect, it } from "vitest";
import { validateSubmission } from "@/lib/validation/submission";

// Submission dates were only checked against /^\d{4}-\d{2}-\d{2}$/, so a day
// such as 2026-02-30 or 2026-13-01 passed Level 1 validation (it also passes
// the string-based future-date and range checks) and only failed when the
// submit route wrote it to the Postgres `date` column, which surfaced as a 500.
// Rejecting impossible calendar days in the schema turns that into a 400.

const tokens = {
  input: 100,
  output: 100,
  cacheRead: 100,
  cacheWrite: 0,
  reasoning: 0,
};

function buildSubmission(
  date: string,
  range: { start: string; end: string } = { start: date, end: date },
  yearRange: { start: string; end: string } = range,
): Record<string, unknown> {
  return {
    meta: {
      generatedAt: "2026-07-14T00:00:00.000Z",
      version: "4.5.3",
      dateRange: range,
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
        year: date.slice(0, 4),
        totalTokens: 300,
        totalCost: 1.5,
        range: yearRange,
      },
    ],
    contributions: [
      {
        date,
        totals: { tokens: 300, cost: 1.5, messages: 0 },
        intensity: 4,
        tokenBreakdown: tokens,
        clients: [{ client: "codex", modelId: "gpt-5.5", tokens, cost: 1.5, messages: 0 }],
      },
    ],
  };
}

describe("validateSubmission calendar dates", () => {
  it.each(["2026-02-30", "2025-02-29", "2026-13-01", "2026-00-10", "2026-04-31"])(
    "rejects the impossible contribution date %s",
    (date) => {
      const result = validateSubmission(
        buildSubmission(date, { start: "2025-01-01", end: "2026-05-31" }),
      );

      expect(result.valid).toBe(false);
      expect(result.errors).toContain("contributions.0.date: Invalid calendar date");
    },
  );

  it("rejects an impossible dateRange bound", () => {
    const result = validateSubmission(
      buildSubmission("2026-02-28", { start: "2026-02-01", end: "2026-02-30" }),
    );

    expect(result.valid).toBe(false);
    expect(result.errors).toContain("meta.dateRange.end: Invalid calendar date");
  });

  it("rejects an impossible year range bound on its own", () => {
    const result = validateSubmission(
      buildSubmission(
        "2026-02-28",
        { start: "2026-02-01", end: "2026-02-28" },
        { start: "2026-02-01", end: "2026-02-30" },
      ),
    );

    expect(result.valid).toBe(false);
    expect(result.errors).toEqual(["years.0.range.end: Invalid calendar date"]);
  });

  it.each(["2024-02-29", "2026-02-28", "2025-12-31"])(
    "still accepts the real calendar day %s",
    (date) => {
      const result = validateSubmission(buildSubmission(date));

      expect(result.errors).toEqual([]);
      expect(result.valid).toBe(true);
    },
  );
});
