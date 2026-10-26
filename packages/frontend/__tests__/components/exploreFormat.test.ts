import { describe, expect, it } from "vitest";
import {
  adoptionNote,
  adoptionShare,
  buildTrendGeometry,
  clampShare,
  describeCoverage,
  describeUsersChange,
  estimatedCostPhrase,
  formatCount,
  formatDateRange,
  formatDay,
  formatEstimatedCost,
  formatShare,
  formatTokens,
  formatUpdatedAt,
  niceCeiling,
  periodLabel,
  resolveExplorePeriod,
  resolveExploreSort,
  sumShares,
  tokenMixParts,
} from "@/components/explore/format";

describe("explore params", () => {
  it("defaults to 30 days and active users", () => {
    expect(resolveExplorePeriod(undefined)).toBe("30d");
    expect(resolveExplorePeriod("week")).toBe("30d");
    expect(resolveExplorePeriod(["7d", "all"])).toBe("7d");
    expect(resolveExplorePeriod(null, "all")).toBe("all");
    expect(resolveExploreSort(undefined)).toBe("users");
    expect(resolveExploreSort("cost")).toBe("users");
    expect(resolveExploreSort("tokens")).toBe("tokens");
  });

  it("labels periods", () => {
    expect(periodLabel("7d")).toBe("Last 7 days");
    expect(periodLabel("all")).toBe("All time");
  });
});

describe("explore number formatting", () => {
  it("formats exact counts", () => {
    expect(formatCount(1406)).toBe("1,406");
    expect(formatCount(-3)).toBe("0");
    expect(formatCount(Number.NaN)).toBe("0");
  });

  it("formats compact token counts", () => {
    expect(formatTokens(950)).toBe("950");
    expect(formatTokens(12_345)).toBe("12.3K");
    expect(formatTokens(950_000)).toBe("950K");
    expect(formatTokens(6_500_000_000_000)).toBe("6.5T");
  });

  it("formats estimated cost without hiding small values", () => {
    expect(formatEstimatedCost(0)).toBe("$0");
    expect(formatEstimatedCost(0.4)).toBe("<$1");
    expect(formatEstimatedCost(512.6)).toBe("$513");
    expect(formatEstimatedCost(3_500_000)).toBe("$3.5M");
  });

  it("shows no price instead of $0 when there is usage but no cost", () => {
    expect(formatEstimatedCost(0, 1_200_000)).toBe("—");
    expect(formatEstimatedCost(0, 0)).toBe("$0");
    expect(formatEstimatedCost(42, 1_200_000)).toBe("$42");
    expect(estimatedCostPhrase(0, 1_200_000)).toBe("no price");
    expect(estimatedCostPhrase(1_500, 1_200_000)).toBe("$1.5K est. cost");
  });

  it("formats shares with sensible precision", () => {
    expect(formatShare(0)).toBe("0%");
    expect(formatShare(0.0004)).toBe("<0.1%");
    expect(formatShare(0.055)).toBe("5.5%");
    expect(formatShare(0.364)).toBe("36%");
    expect(formatShare(1.4)).toBe("100%");
    expect(formatShare(Number.NaN)).toBe("0%");
  });
});

describe("adoption math", () => {
  it("divides active users by accounts and guards empty periods", () => {
    expect(adoptionShare(360, 1000)).toBeCloseTo(0.36);
    expect(adoptionShare(5, 0)).toBe(0);
    expect(adoptionShare(12, 10)).toBe(1);
    expect(clampShare(-0.2)).toBe(0);
  });

  it("lets shares across models add up past 100%", () => {
    expect(sumShares([0.36, 0.35, 0.21, 0.2])).toBeCloseTo(1.12);
    expect(adoptionNote([0.36, 0.35, 0.21, 0.2], 1406, "model")).toContain("add up to 112%, not 100%");
    expect(adoptionNote([0.2, 0.1], 1406, "agent")).toContain("can add up to more than 100%");
    expect(adoptionNote([0.2], 1, "model")).toContain("the 1 account active");
  });
});

describe("describeUsersChange", () => {
  it("shows nothing for all time", () => {
    const change = describeUsersChange(512, null, false, "all");
    expect(change.kind).toBe("none");
    expect(change.text).toBe("");
  });

  it("marks models without previous users as new and shows added users", () => {
    const change = describeUsersChange(142, 0, true, "30d");
    expect(change.kind).toBe("new");
    expect(change.text).toBe("+142");
    expect(change.percentText).toBeNull();
    expect(change.description).toContain("previous 30 days");
  });

  it("treats a zero previous window as new even when the flag is missing", () => {
    expect(describeUsersChange(9, 0, false, "7d").kind).toBe("new");
  });

  it("adds a percentage only on a large enough base", () => {
    const big = describeUsersChange(512, 470, false, "30d");
    expect(big.kind).toBe("up");
    expect(big.text).toBe("+42");
    expect(big.percentText).toBe("+8.9%");

    const small = describeUsersChange(30, 3, false, "7d");
    expect(small.text).toBe("+27");
    expect(small.percentText).toBeNull();
  });

  it("never shows a percentage of 1000% or more", () => {
    const change = describeUsersChange(4000, 300, false, "30d");
    expect(change.text).toBe("+3,700");
    expect(change.percentText).toBeNull();
  });

  it("uses a real minus sign for losses", () => {
    const change = describeUsersChange(280, 301, false, "30d");
    expect(change.kind).toBe("down");
    expect(change.text).toBe("\u221221");
    expect(change.percentText).toBe("\u22127.0%");
    expect(change.description).toBe("Down 21 from 301 active users in the previous 30 days.");
  });

  it("reports no change", () => {
    const change = describeUsersChange(50, 50, false, "7d");
    expect(change.kind).toBe("flat");
    expect(change.text).toBe("No change");
  });
});

describe("explore dates", () => {
  it("formats days and ranges in UTC without the locale", () => {
    expect(formatDay("2026-10-18")).toBe("Oct 18, 2026");
    expect(formatDay("not a date")).toBe("not a date");
    expect(formatDateRange("2026-10-12", "2026-10-18")).toBe("Oct 12\u201318, 2026");
    expect(formatDateRange("2026-09-19", "2026-10-18")).toBe("Sep 19 \u2013 Oct 18, 2026");
    expect(formatDateRange("2025-09-14", "2026-10-18")).toBe("Sep 14, 2025 \u2013 Oct 18, 2026");
    expect(formatDateRange("2026-10-18", "2026-10-18")).toBe("Oct 18, 2026");
  });

  it("formats the data time in UTC", () => {
    expect(formatUpdatedAt("2026-10-19T04:12:00.000Z")).toBe("Oct 19, 2026, 04:12 UTC");
    expect(formatUpdatedAt(null)).toBeNull();
    expect(formatUpdatedAt("garbage")).toBeNull();
  });

  it("describes coverage", () => {
    const facts = describeCoverage({
      period: "30d",
      start: "2026-09-19",
      end: "2026-10-18",
      accounts: 1406,
      modelIdentifiedShare: 0.987,
      lastDataAt: "2026-10-19T04:12:00.000Z",
    });
    expect(facts).toEqual({
      range: "Sep 19 \u2013 Oct 18, 2026 (UTC)",
      accounts: "1,406 accounts",
      identified: "99% of tokens from an identified model",
      updated: "Oct 19, 2026, 04:12 UTC",
    });
  });
});

describe("tokenMixParts", () => {
  it("normalises shares and absolute counts the same way", () => {
    const fromShares = tokenMixParts({ input: 0.031, output: 0.004, cacheRead: 0.93, cacheWrite: 0.034, reasoning: 0.001 });
    const fromCounts = tokenMixParts({ input: 31, output: 4, cacheRead: 930, cacheWrite: 34, reasoning: 1 });
    expect(fromShares.map((p) => p.key)).toEqual(["input", "output", "cacheRead", "cacheWrite", "reasoning"]);
    fromShares.forEach((part, index) => expect(part.share).toBeCloseTo(fromCounts[index].share));
    expect(fromCounts.reduce((sum, part) => sum + part.share, 0)).toBeCloseTo(1);
  });

  it("returns zero shares for an empty mix", () => {
    const parts = tokenMixParts({ input: 0, output: 0, cacheRead: 0, cacheWrite: 0, reasoning: 0 });
    expect(parts.every((part) => part.share === 0)).toBe(true);
  });
});

describe("trend geometry", () => {
  it("rounds the scale up to a readable value", () => {
    expect(niceCeiling(0)).toBe(0);
    expect(niceCeiling(7)).toBe(8);
    expect(niceCeiling(342)).toBe(400);
    expect(niceCeiling(1234)).toBe(1500);
    expect(niceCeiling(9100)).toBe(10000);
  });

  it("maps values into the box with y growing down", () => {
    const geometry = buildTrendGeometry([0, 50, 100], 200, 100);
    expect(geometry.max).toBe(100);
    expect(geometry.points.map((p) => [p.x, p.y])).toEqual([
      [0, 100],
      [100, 50],
      [200, 0],
    ]);
    expect(geometry.line).toBe("M0 100 L100 50 L200 0");
    expect(geometry.area.endsWith("L200 100 L0 100 Z")).toBe(true);
  });

  it("handles empty, single and all-zero series", () => {
    expect(buildTrendGeometry([], 100, 50).line).toBe("");
    expect(buildTrendGeometry([5], 100, 50).points[0].x).toBe(50);
    expect(buildTrendGeometry([0, 0], 100, 50).points.every((p) => p.y === 50)).toBe(true);
  });
});
