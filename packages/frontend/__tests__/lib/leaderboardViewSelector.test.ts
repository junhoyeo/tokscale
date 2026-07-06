import { describe, expect, it } from "vitest";
import { buildLeaderboardViewHref } from "@/app/(main)/leaderboard/ViewSelector";

describe("leaderboard view selector links", () => {
  it("preserves filters while dropping pagination when switching views", () => {
    const href = buildLeaderboardViewHref(
      {
        view: "users",
        period: "custom",
        from: "2026-01-01",
        to: "2026-01-31",
        sortBy: "time",
        search: "alice",
        page: "3",
      },
      "groups"
    );

    const url = new URL(href, "http://localhost");
    expect(url.pathname).toBe("/leaderboard");
    expect(url.searchParams.get("view")).toBe("groups");
    expect(url.searchParams.get("period")).toBe("custom");
    expect(url.searchParams.get("from")).toBe("2026-01-01");
    expect(url.searchParams.get("to")).toBe("2026-01-31");
    expect(url.searchParams.get("sortBy")).toBe("time");
    expect(url.searchParams.get("search")).toBe("alice");
    expect(url.searchParams.has("page")).toBe(false);
  });
});
