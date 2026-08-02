import { describe, expect, it } from "vitest";
import {
  isValidCustomDateRange,
  isValidDateString,
  parseCustomDateRange,
} from "@/lib/leaderboard/dateRange";

describe("leaderboard date range helpers", () => {
  it("validates real calendar dates", () => {
    expect(isValidDateString("2024-02-29")).toBe(true);
    expect(isValidDateString("2026-02-31")).toBe(false);
    expect(isValidDateString("2026-13-01")).toBe(false);
    expect(isValidDateString("2026-1-01")).toBe(false);
  });

  it("returns false when custom range is inverted", () => {
    expect(isValidCustomDateRange("2026-02-10", "2026-02-09")).toBe(false);
  });

  it("parses valid custom ranges and rejects invalid", () => {
    expect(parseCustomDateRange("2026-02-01", "2026-02-28")).toEqual({
      from: "2026-02-01",
      to: "2026-02-28",
    });

    expect(parseCustomDateRange("2026-02-31", "2026-02-28")).toBeNull();
    expect(parseCustomDateRange("2026-02-10", "2026-02-09")).toBeNull();
  });
});
