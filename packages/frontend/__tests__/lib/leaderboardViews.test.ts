import { describe, expect, it } from "vitest";
import {
  buildExploreHref,
  buildLeaderboardViewHref,
  resolveLeaderboardView,
} from "@/app/(main)/leaderboard/views";

function parse(href: string) {
  return new URL(href, "http://localhost");
}

describe("resolveLeaderboardView", () => {
  it("keeps old ?view=users links on the humans view", () => {
    expect(resolveLeaderboardView("users")).toBe("humans");
    expect(resolveLeaderboardView(undefined)).toBe("humans");
    expect(resolveLeaderboardView("nonsense")).toBe("humans");
    expect(resolveLeaderboardView("humans")).toBe("humans");
  });

  it("resolves the other views", () => {
    expect(resolveLeaderboardView("models")).toBe("models");
    expect(resolveLeaderboardView("agents")).toBe("agents");
    expect(resolveLeaderboardView(["groups", "models"])).toBe("groups");
  });
});

describe("buildLeaderboardViewHref", () => {
  const humanFilters = {
    view: "users",
    period: "custom",
    from: "2026-01-01",
    to: "2026-01-31",
    sortBy: "cost",
    search: "alice",
    page: "3",
  };

  it("links humans to the bare /leaderboard URL with its filters", () => {
    const url = parse(buildLeaderboardViewHref(humanFilters, "humans"));
    expect(url.pathname).toBe("/leaderboard");
    expect(url.searchParams.has("view")).toBe(false);
    expect(url.searchParams.get("sortBy")).toBe("cost");
    expect(url.searchParams.get("from")).toBe("2026-01-01");
    expect(url.searchParams.has("page")).toBe(false);
    expect(buildLeaderboardViewHref({}, "humans")).toBe("/leaderboard");
  });

  it("drops humans-only filters when moving to models or agents", () => {
    const url = parse(buildLeaderboardViewHref(humanFilters, "models"));
    expect(url.searchParams.get("view")).toBe("models");
    for (const key of ["period", "from", "to", "sortBy", "search", "page"]) {
      expect(url.searchParams.has(key)).toBe(false);
    }
  });

  it("carries explore period and sort between models and agents", () => {
    const url = parse(buildLeaderboardViewHref({ view: "models", period: "7d", sort: "tokens" }, "agents"));
    expect(url.searchParams.get("view")).toBe("agents");
    expect(url.searchParams.get("period")).toBe("7d");
    expect(url.searchParams.get("sort")).toBe("tokens");
  });

  it("drops explore-only values when moving back to humans", () => {
    const url = parse(buildLeaderboardViewHref({ view: "models", period: "7d", sort: "tokens" }, "humans"));
    expect(url.search).toBe("");
  });

  it("keeps a period both views understand", () => {
    expect(buildLeaderboardViewHref({ view: "models", period: "all" }, "humans")).toBe("/leaderboard?period=all");
    expect(buildLeaderboardViewHref({ period: "all" }, "agents")).toBe("/leaderboard?period=all&view=agents");
  });

  it("drops from/to unless the period is custom", () => {
    const url = parse(buildLeaderboardViewHref({ period: "week", from: "2026-01-01", to: "2026-01-31" }, "groups"));
    expect(url.searchParams.get("period")).toBe("week");
    expect(url.searchParams.has("from")).toBe(false);
  });
});

describe("buildExploreHref", () => {
  const defaults = { period: "30d", sort: "users" };

  it("leaves defaults out of the URL", () => {
    expect(buildExploreHref("/leaderboard", { view: "models", period: "7d", sort: "users" }, { period: "30d" }, defaults)).toBe(
      "/leaderboard?view=models",
    );
  });

  it("keeps the other control when one changes", () => {
    expect(buildExploreHref("/leaderboard", { view: "agents", period: "7d", sort: "users" }, { sort: "tokens" }, defaults)).toBe(
      "/leaderboard?view=agents&period=7d&sort=tokens",
    );
  });

  it("works for pages without a view", () => {
    expect(buildExploreHref("/models/claude-opus-5", { period: "30d" }, { period: "all" }, { period: "30d" })).toBe(
      "/models/claude-opus-5?period=all",
    );
    expect(buildExploreHref("/models", { period: "all", sort: "tokens" }, { period: "all" }, { period: "all", sort: "users" })).toBe(
      "/models?sort=tokens",
    );
  });
});
