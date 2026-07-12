import { describe, expect, it } from "vitest";
import {
  formatGroupMemberCount,
  getGroupMonogram,
  getLeaderboardPeriodLabel,
} from "@/components/leaderboard/presentation";

describe("leaderboard presentation helpers", () => {
  it("builds stable group monograms from one-word and multi-word names", () => {
    expect(getGroupMonogram("Team Australia")).toBe("TA");
    expect(getGroupMonogram("system")).toBe("SY");
    expect(getGroupMonogram("한국 AI 토큰 경쟁")).toBe("한A");
    expect(getGroupMonogram("  ")).toBe("TG");
  });

  it("uses correct member-count grammar", () => {
    expect(formatGroupMemberCount(0)).toBe("0 members");
    expect(formatGroupMemberCount(1)).toBe("1 member");
    expect(formatGroupMemberCount(2)).toBe("2 members");
  });

  it("describes predefined and custom leaderboard periods", () => {
    expect(getLeaderboardPeriodLabel("all")).toBe("All time");
    expect(getLeaderboardPeriodLabel("last-month")).toBe("Last month");
    expect(getLeaderboardPeriodLabel("month")).toBe("This month");
    expect(getLeaderboardPeriodLabel("week")).toBe("This week");
    expect(
      getLeaderboardPeriodLabel("custom", "2026-06-01", "2026-06-30"),
    ).toBe("Jun 1–30, 2026");
    expect(getLeaderboardPeriodLabel("custom")).toBe("Custom range");
  });
});
