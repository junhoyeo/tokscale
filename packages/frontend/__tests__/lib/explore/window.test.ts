import { describe, expect, it } from "vitest";
import {
  addDays,
  adoption,
  daysBetweenInclusive,
  enumerateDates,
  identifiedShare,
  isNewInWindow,
  maxAnchorDate,
  queryStart,
  resolveWindow,
} from "@/lib/explore/window";

const bounds = { firstDate: "2025-04-16", latestDate: "2026-10-19" };

describe("resolveWindow", () => {
  it("ends 7d at the latest date with data and keeps a same-length previous window", () => {
    const w = resolveWindow("7d", bounds, "2026-12-31");
    expect(w).toEqual({
      period: "7d",
      start: "2026-10-13",
      end: "2026-10-19",
      previous: { start: "2026-10-06", end: "2026-10-12" },
      days: 7,
    });
    expect(daysBetweenInclusive(w.previous!.start, w.previous!.end)).toBe(7);
    expect(queryStart(w)).toBe("2026-10-06");
  });

  it("handles 30d across a month boundary", () => {
    const w = resolveWindow("30d", { firstDate: "2025-01-01", latestDate: "2026-03-01" }, "2026-03-01");
    expect(w.start).toBe("2026-01-31");
    expect(w.previous).toEqual({ start: "2026-01-01", end: "2026-01-30" });
    expect(daysBetweenInclusive(w.start, w.end)).toBe(30);
  });

  it("starts 'all' at the first date with data and has no previous window", () => {
    const w = resolveWindow("all", bounds, "2026-12-31");
    expect(w.start).toBe("2025-04-16");
    expect(w.end).toBe("2026-10-19");
    expect(w.previous).toBeNull();
    expect(w.days).toBe(552);
    expect(queryStart(w)).toBe("2025-04-16");
  });

  it("falls back to the given end date when there is no data", () => {
    const all = resolveWindow("all", { firstDate: null, latestDate: null }, "2026-10-19");
    expect(all).toMatchObject({ start: "2026-10-19", end: "2026-10-19", days: 1, previous: null });
    const week = resolveWindow("7d", { firstDate: null, latestDate: null }, "2026-10-19");
    expect(week.start).toBe("2026-10-13");
  });
});

describe("date helpers", () => {
  it("adds days in UTC, including leap days", () => {
    expect(addDays("2028-02-28", 1)).toBe("2028-02-29");
    expect(addDays("2026-03-01", -1)).toBe("2026-02-28");
  });

  it("enumerates inclusive ranges", () => {
    expect(enumerateDates("2026-10-17", "2026-10-19")).toEqual(["2026-10-17", "2026-10-18", "2026-10-19"]);
    expect(enumerateDates("2026-10-19", "2026-10-18")).toEqual([]);
  });

  it("allows an anchor at most one day ahead of UTC today", () => {
    expect(maxAnchorDate(new Date("2026-10-19T23:59:00Z"))).toBe("2026-10-20");
    expect(maxAnchorDate(new Date("2026-10-19T00:00:00Z"))).toBe("2026-10-20");
  });

  it("rejects malformed dates", () => {
    expect(() => addDays("2026/10/19", 1)).toThrow();
  });
});

describe("coverage math", () => {
  it("computes adoption against visible accounts", () => {
    expect(adoption(50, 200)).toBe(0.25);
    expect(adoption(3, 0)).toBe(0);
  });

  it("clamps the identified share to 0..1", () => {
    expect(identifiedShare(990, 1000)).toBe(0.99);
    expect(identifiedShare(0, 0)).toBe(0);
    expect(identifiedShare(1200, 1000)).toBe(1);
  });

  it("marks a model new only when its first use is inside the window", () => {
    const w = resolveWindow("7d", bounds, "2026-10-19");
    expect(isNewInWindow(w, "2026-10-13")).toBe(true);
    expect(isNewInWindow(w, "2026-10-12")).toBe(false);
    expect(isNewInWindow(w, null)).toBe(false);
    expect(isNewInWindow(resolveWindow("all", bounds, "2026-10-19"), "2026-10-19")).toBe(false);
  });
});
