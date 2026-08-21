import { describe, expect, it } from "vitest";
import { validateSubmission } from "@/lib/validation/submission";

// A repeated date in `contributions` used to reach the submit route intact.
// The route builds its INSERT batch from a date-keyed map of EXISTING rows that
// is never updated as the loop runs, so both entries miss the map and both are
// queued as inserts -- two rows sharing (submission_id, submitted_device_id,
// date). Postgres then either aborts the statement ("ON CONFLICT DO UPDATE
// command cannot affect row a second time") or, when the duplicates straddle an
// INSERT_CHUNK_SIZE boundary, lets the second row overwrite the first through
// the ON CONFLICT arm -- which is the one path that bypasses the monotonic
// active-time guard. Rejecting at validation turns both into a clean 400.

const tokens = {
  input: 100,
  output: 100,
  cacheRead: 100,
  cacheWrite: 0,
  reasoning: 0,
};

function buildDay(date: string): Record<string, unknown> {
  return {
    date,
    totals: { tokens: 300, cost: 1.5, messages: 0 },
    intensity: 4,
    tokenBreakdown: tokens,
    clients: [{ client: "codex", modelId: "gpt-5.5", tokens, cost: 1.5, messages: 0 }],
  };
}

function buildSubmission(dates: string[]): Record<string, unknown> {
  return {
    meta: {
      generatedAt: "2026-07-14T00:00:00.000Z",
      version: "4.5.3",
      dateRange: { start: "2026-05-11", end: "2026-05-11" },
    },
    summary: {
      totalTokens: 300 * dates.length,
      totalCost: 1.5 * dates.length,
      totalDays: dates.length,
      activeDays: dates.length,
      averagePerDay: 1.5,
      maxCostInSingleDay: 1.5,
      clients: ["codex"],
      models: ["gpt-5.5"],
    },
    years: [
      {
        year: "2026",
        totalTokens: 300 * dates.length,
        totalCost: 1.5 * dates.length,
        range: { start: "2026-05-11", end: "2026-05-11" },
      },
    ],
    contributions: dates.map(buildDay),
  };
}

const DUPLICATE_MESSAGE = "Duplicate dates in contributions";

describe("submission duplicate-date guard", () => {
  it("accepts distinct contribution dates", () => {
    const result = validateSubmission(buildSubmission(["2026-05-11"]));

    expect(result.errors).not.toEqual(
      expect.arrayContaining([expect.stringContaining(DUPLICATE_MESSAGE)]),
    );
  });

  it("rejects a submission repeating the same contribution date", () => {
    const result = validateSubmission(
      buildSubmission(["2026-05-11", "2026-05-11"]),
    );

    expect(result.valid).toBe(false);
    expect(result.errors).toEqual(
      expect.arrayContaining([expect.stringContaining(DUPLICATE_MESSAGE)]),
    );
  });

  it("rejects duplicates that are not adjacent in the array", () => {
    const result = validateSubmission(
      buildSubmission(["2026-05-11", "2026-05-12", "2026-05-11"]),
    );

    expect(result.valid).toBe(false);
    expect(result.errors).toEqual(
      expect.arrayContaining([expect.stringContaining(DUPLICATE_MESSAGE)]),
    );
  });
});
