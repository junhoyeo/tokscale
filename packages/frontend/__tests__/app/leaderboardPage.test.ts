import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

const cookies = vi.fn();
const getLeaderboardData = vi.fn();
const getUserRank = vi.fn();
const getSession = vi.fn();

vi.mock("next/headers", () => ({
  cookies,
}));

vi.mock("@/lib/leaderboard/getLeaderboard", () => ({
  getLeaderboardData,
  getUserRank,
}));

vi.mock("@/lib/auth/session", () => ({
  getSession,
}));

vi.mock("@/components/layout/Navigation", () => ({
  Navigation: function Navigation() {
    return null;
  },
}));

vi.mock("@/components/layout/Footer", () => ({
  Footer: function Footer() {
    return null;
  },
}));

vi.mock("@/components/BlackholeHero", () => ({
  BlackholeHero: function BlackholeHero() {
    return null;
  },
}));

vi.mock("@/components/Skeleton", () => ({
  LeaderboardSkeleton: function LeaderboardSkeleton() {
    return null;
  },
}));

vi.mock("../../src/app/(main)/leaderboard/LeaderboardClient", () => ({
  default: function LeaderboardClient() {
    return null;
  },
}));

vi.mock("@/lib/leaderboard/constants", () => ({
  SORT_BY_COOKIE_NAME: "leaderboard-sort-by",
  resolveSortByParam: (value: unknown) =>
    value === "tokens" || value === "cost" ? value : null,
}));

type LeaderboardPageModule = typeof import("../../src/app/(main)/leaderboard/page");

let loadLeaderboardPageData: LeaderboardPageModule["loadLeaderboardPageData"];

beforeAll(async () => {
  const pageModule = await import("../../src/app/(main)/leaderboard/page");
  loadLeaderboardPageData = pageModule.loadLeaderboardPageData;
});

beforeEach(() => {
  cookies.mockReset();
  getLeaderboardData.mockReset();
  getUserRank.mockReset();
  getSession.mockReset();
});

describe("loadLeaderboardPageData", () => {
  it("loads the same trusted all-time leaderboard slice and current-user rank as the public API surface", async () => {
    const cookieStore = {
      get: vi.fn((name: string) =>
        name === "leaderboard-sort-by" ? { value: "cost" } : undefined
      ),
    };
    const initialData = {
      users: [
        {
          rank: 1,
          userId: "user-1",
          username: "alice",
          displayName: "Alice",
          avatarUrl: null,
          totalTokens: 1200,
          totalCost: 12,
          submissionCount: 1,
          lastSubmission: "2026-04-18T00:00:00.000Z",
          submissionFreshness: null,
        },
      ],
      pagination: {
        page: 1,
        limit: 50,
        totalUsers: 1,
        totalPages: 1,
        hasNext: false,
        hasPrev: false,
      },
      stats: {
        totalTokens: 1200,
        totalCost: 12,
        totalSubmissions: 1,
        uniqueUsers: 1,
      },
      period: "all" as const,
      sortBy: "cost" as const,
    };
    const currentUser = {
      id: "user-1",
      username: "alice",
      displayName: "Alice",
      avatarUrl: null,
      isAdmin: false,
    };
    const currentUserRank = {
      ...initialData.users[0],
    };

    cookies.mockResolvedValue(cookieStore);
    getLeaderboardData.mockResolvedValue(initialData);
    getSession.mockResolvedValue(currentUser);
    getUserRank.mockResolvedValue(currentUserRank);

    const result = await loadLeaderboardPageData();

    expect(getLeaderboardData).toHaveBeenCalledWith(
      "all",
      1,
      50,
      "cost",
      "",
      undefined,
      undefined
    );
    expect(getUserRank).toHaveBeenCalledWith(
      "alice",
      "all",
      "cost",
      undefined,
      undefined
    );
    expect(result).toEqual({
      initialData,
      initialSortBy: "cost",
      initialUserRank: currentUserRank,
      session: currentUser,
    });
  });
});
