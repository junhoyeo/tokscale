import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

const getLeaderboardData = vi.fn();

vi.mock("@/lib/leaderboard/getLeaderboard", () => ({
  getLeaderboardData,
}));

type ModuleExports = typeof import("../../src/app/api/leaderboard/route");

let GET: ModuleExports["GET"];

beforeAll(async () => {
  const routeModule = await import("../../src/app/api/leaderboard/route");
  GET = routeModule.GET;
});

beforeEach(() => {
  getLeaderboardData.mockReset();
});

describe("GET /api/leaderboard", () => {
  it("passes submission freshness metadata through unchanged", async () => {
    getLeaderboardData.mockResolvedValue({
      users: [
        {
          rank: 1,
          userId: "user-1",
          username: "alice",
          displayName: "Alice",
          avatarUrl: null,
          totalTokens: 1200,
          totalCost: 12.5,
          submissionCount: 2,
          lastSubmission: "2026-01-10T10:00:00.000Z",
          submissionFreshness: {
            lastUpdated: "2026-01-10T10:00:00.000Z",
            cliVersion: "1.4.2",
            schemaVersion: 1,
            isStale: true,
          },
        },
      ],
      pagination: {
        page: 1,
        limit: 10,
        totalUsers: 1,
        totalPages: 1,
        hasNext: false,
        hasPrev: false,
      },
      stats: {
        totalTokens: 1200,
        totalCost: 12.5,
        totalSubmissions: 1,
        uniqueUsers: 1,
      },
      period: "all",
      sortBy: "tokens",
    });

    const response = await GET(
      new Request("http://localhost:3000/api/leaderboard?period=all&page=1&limit=10&sortBy=tokens")
    );
    const body = await response.json();

    expect(response.status).toBe(200);
    expect(getLeaderboardData).toHaveBeenCalledWith("all", 1, 10, "tokens");
    expect(body.users[0].submissionFreshness).toEqual({
      lastUpdated: "2026-01-10T10:00:00.000Z",
      cliVersion: "1.4.2",
      schemaVersion: 1,
      isStale: true,
    });
  });
});
