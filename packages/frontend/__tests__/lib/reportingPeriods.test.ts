import { describe, expect, it } from "vitest";
import { getCalendarDateRange } from "../../src/lib/reportingPeriods";

describe("reporting calendar", () => {
  it.each([
    ["2026-01-01T12:00:00Z", "2025-12-01", "2025-12-31"],
    ["2024-03-01T12:00:00Z", "2024-02-01", "2024-02-29"],
    ["2026-03-01T12:00:00Z", "2026-02-01", "2026-02-28"],
  ])("resolves the completed month at %s", (now, start, end) => {
    expect(getCalendarDateRange("last-month", new Date(now))).toEqual({ start, end });
  });

  it("resolves local calendar dates before calculating month boundaries", () => {
    const now = new Date("2026-03-01T00:30:00Z");
    expect(getCalendarDateRange("month", now, "America/Los_Angeles")).toEqual({ start: "2026-02-01", end: "2026-02-28" });
    expect(getCalendarDateRange("month", now, "Asia/Seoul")).toEqual({ start: "2026-03-01", end: "2026-03-01" });
  });

  it.each([
    [0, "2026-03-08"], [1, "2026-03-02"], [2, "2026-03-03"],
    [3, "2026-03-04"], [4, "2026-03-05"], [5, "2026-03-06"], [6, "2026-03-07"],
  ])("starts the current week on weekday %i", (weekStartsOn, start) => {
    expect(getCalendarDateRange("week", new Date("2026-03-08T12:00:00Z"), "UTC", weekStartsOn)).toEqual({ start, end: "2026-03-08" });
  });

  it("selects a completed week across year boundaries", () => {
    expect(getCalendarDateRange("last-week", new Date("2026-01-01T12:00:00Z"))).toEqual({ start: "2025-12-22", end: "2025-12-28" });
  });

  it("follows local midnight across daylight-saving changes", () => {
    expect(getCalendarDateRange("week", new Date("2026-03-09T03:30:00Z"), "America/New_York")).toEqual({ start: "2026-03-02", end: "2026-03-08" });
    expect(getCalendarDateRange("week", new Date("2026-03-09T04:30:00Z"), "America/New_York")).toEqual({ start: "2026-03-09", end: "2026-03-09" });
  });
});
