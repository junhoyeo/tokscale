import { Children, type ReactElement, isValidElement } from "react";
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

const getStargazersCount = vi.fn();
const getLeaderboardData = vi.fn();

vi.mock("@/lib/github", () => ({
  getStargazersCount,
}));

vi.mock("@/lib/leaderboard/getLeaderboard", () => ({
  getLeaderboardData,
}));

vi.mock("@/components/layout/Navigation", () => ({
  Navigation: function Navigation() {
    return null;
  },
}));

vi.mock("@/components/landing/LandingPage", () => ({
  LandingPage: function LandingPage() {
    return null;
  },
}));

type HomePageModule = typeof import("../../src/app/(main)/page");

let HomePage: HomePageModule["default"];

function createLeaderboardData(sortBy: "tokens" | "cost", users: unknown[]) {
  return {
    users,
    pagination: {
      page: 1,
      limit: 5,
      totalUsers: users.length,
      totalPages: 1,
      hasNext: false,
      hasPrev: false,
    },
    stats: {
      totalTokens: 1000,
      totalCost: 10,
      totalSubmissions: 1,
      uniqueUsers: users.length,
    },
    period: "all" as const,
    sortBy,
  };
}

beforeAll(async () => {
  const pageModule = await import("../../src/app/(main)/page");
  HomePage = pageModule.default;
});

beforeEach(() => {
  getStargazersCount.mockReset();
  getLeaderboardData.mockReset();
});

describe("HomePage", () => {
  it("passes the trusted leaderboard winners to the landing-page top-user widgets", async () => {
    const topUsersByCost = [
      {
        rank: 1,
        userId: "user-cost",
        username: "cost-king",
        displayName: "Cost King",
        avatarUrl: null,
        totalTokens: 900,
        totalCost: 90,
        submissionCount: 1,
        lastSubmission: "2026-04-18T00:00:00.000Z",
        submissionFreshness: null,
      },
    ];
    const topUsersByTokens = [
      {
        rank: 1,
        userId: "user-tokens",
        username: "token-queen",
        displayName: "Token Queen",
        avatarUrl: null,
        totalTokens: 5000,
        totalCost: 50,
        submissionCount: 1,
        lastSubmission: "2026-04-18T00:00:00.000Z",
        submissionFreshness: null,
      },
    ];

    getStargazersCount.mockResolvedValue(42);
    getLeaderboardData
      .mockResolvedValueOnce(createLeaderboardData("cost", topUsersByCost))
      .mockResolvedValueOnce(createLeaderboardData("tokens", topUsersByTokens));

    const page = await HomePage();

    expect(getLeaderboardData).toHaveBeenNthCalledWith(1, "all", 1, 5, "cost");
    expect(getLeaderboardData).toHaveBeenNthCalledWith(2, "all", 1, 5, "tokens");
    expect(isValidElement(page)).toBe(true);

    const children = Children.toArray(page.props.children);
    const landingPage = children[1] as ReactElement<{
      stargazersCount: number;
      topUsersByCost: typeof topUsersByCost;
      topUsersByTokens: typeof topUsersByTokens;
    }>;

    expect(isValidElement(landingPage)).toBe(true);
    if (!isValidElement(landingPage)) {
      throw new Error("LandingPage element missing");
    }

    expect(landingPage.props.stargazersCount).toBe(42);
    expect(landingPage.props.topUsersByCost).toEqual(topUsersByCost);
    expect(landingPage.props.topUsersByTokens).toEqual(topUsersByTokens);
  });
});
