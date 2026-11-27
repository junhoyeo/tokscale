import { describe, expect, it, vi } from "vitest";
import {
  getPeriodSubmissionStats,
  recordDailySubmissionCount,
  resolvePeriodSubmissionStats,
  toUtcSubmissionDate,
} from "../../src/lib/db/submissionDailyCounts";

function flattenSqlStrings(node: unknown, out: string[] = []): string[] {
  if (typeof node === "string") {
    out.push(node);
    return out;
  }
  if (!node || typeof node !== "object") return out;
  if (Array.isArray(node)) {
    for (const item of node) flattenSqlStrings(item, out);
    return out;
  }
  for (const value of Object.values(node as Record<string, unknown>)) {
    flattenSqlStrings(value, out);
  }
  return out;
}

describe("submissionDailyCounts helpers", () => {
  it("extracts the UTC calendar date from an instant", () => {
    expect(toUtcSubmissionDate(new Date("2026-11-27T23:59:59.999Z"))).toBe(
      "2026-11-27",
    );
    expect(toUtcSubmissionDate(new Date("2026-11-28T00:00:00.000Z"))).toBe(
      "2026-11-28",
    );
  });

  it("returns the tracked count when positive", () => {
    expect(resolvePeriodSubmissionStats(14, "2026-11-10", "2026-11-01")).toEqual({
      count: 14,
      trackingStartDate: "2026-11-10",
    });
  });

  it("returns 0 when tracking already covered the window start", () => {
    expect(resolvePeriodSubmissionStats(0, "2026-10-15", "2026-11-01")).toEqual({
      count: 0,
      trackingStartDate: "2026-10-15",
    });
    expect(resolvePeriodSubmissionStats(0, "2026-11-01", "2026-11-01")).toEqual({
      count: 0,
      trackingStartDate: "2026-11-01",
    });
  });

  it("returns null when the window predates daily tracking so historical months do not show a false 0", () => {
    expect(resolvePeriodSubmissionStats(0, null, "2026-10-01")).toEqual({
      count: null,
      trackingStartDate: null,
    });
    expect(resolvePeriodSubmissionStats(0, "2026-11-27", "2026-10-01")).toEqual({
      count: null,
      trackingStartDate: "2026-11-27",
    });
  });

  it("executes an atomic upsert for recordDailySubmissionCount", async () => {
    const execute = vi.fn(async (query: unknown) => {
      void query;
      return [];
    });
    await recordDailySubmissionCount(
      { execute } as unknown as Parameters<typeof recordDailySubmissionCount>[0],
      "sub-123",
      new Date("2026-11-27T16:40:00.000Z"),
    );

    expect(execute).toHaveBeenCalledTimes(1);
    const fragments = flattenSqlStrings(execute.mock.calls[0]?.[0]).join(" ");
    expect(fragments).toContain("INSERT INTO submission_daily_counts");
    expect(fragments).toContain("ON CONFLICT (submission_id, date)");
    expect(fragments).toContain("sub-123");
    expect(fragments).toContain("2026-11-27");
  });

  it("queries and normalizes period submission counts from submission_daily_counts", async () => {
    const execute = vi.fn(async (query: unknown) => {
      void query;
      return [{ period_count: "9", earliest_tracked_date: "2026-11-05" }];
    });

    const result = await getPeriodSubmissionStats(
      "sub-123",
      "2026-11-01",
      "2026-11-30",
      { execute } as unknown as Parameters<typeof getPeriodSubmissionStats>[3],
    );

    expect(result).toEqual({
      count: 9,
      trackingStartDate: "2026-11-05",
    });
    const fragments = flattenSqlStrings(execute.mock.calls[0]?.[0]).join(" ");
    expect(fragments).toContain("FROM submission_daily_counts");
    expect(fragments).toContain("sub-123");
    expect(fragments).toContain("2026-11-01");
    expect(fragments).toContain("2026-11-30");
  });
});
