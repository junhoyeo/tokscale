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
    // #522 added customFrom/customTo as the 6th and 7th positional args; this
    // request doesn't pass them, so they arrive as `undefined`.
    expect(getLeaderboardData).toHaveBeenCalledWith(
      "all",
      1,
      10,
      "tokens",
      "",
      undefined,
      undefined,
    );
    expect(body.users[0].submissionFreshness).toEqual({
      lastUpdated: "2026-01-10T10:00:00.000Z",
      cliVersion: "1.4.2",
      schemaVersion: 1,
      isStale: true,
    });
  });

  it("accepts time sort requests", async () => {
    getLeaderboardData.mockResolvedValue({
      users: [],
      pagination: {
        page: 1,
        limit: 50,
        totalUsers: 0,
        totalPages: 0,
        hasNext: false,
        hasPrev: false,
      },
      stats: {
        totalTokens: 0,
        totalCost: 0,
        totalActiveTimeMs: 0,
        totalSubmissions: 0,
        uniqueUsers: 0,
      },
      period: "all",
      sortBy: "time",
    });

    const response = await GET(
      new Request("http://localhost:3000/api/leaderboard?period=all&sortBy=time")
    );

    expect(response.status).toBe(200);
    expect(getLeaderboardData).toHaveBeenCalledWith(
      "all",
      1,
      50,
      "time",
      "",
      undefined,
      undefined,
    );
  });

  it("uses search query when provided", async () => {
    getLeaderboardData.mockResolvedValue({
      users: [],
      pagination: {
        page: 1,
        limit: 50,
        totalUsers: 0,
        totalPages: 0,
        hasNext: false,
        hasPrev: false,
      },
      stats: {
        totalTokens: 0,
        totalCost: 0,
        totalActiveTimeMs: 0,
        totalSubmissions: 0,
        uniqueUsers: 0,
      },
      period: "all",
      sortBy: "tokens",
    });

    const response = await GET(
      new Request("http://localhost:3000/api/leaderboard?search=alice")
    );

    expect(response.status).toBe(200);
    expect(getLeaderboardData).toHaveBeenCalledWith(
      "all",
      1,
      50,
      "tokens",
      "alice",
      undefined,
      undefined,
    );
  });

  it("falls back to all period for invalid custom date ranges", async () => {
    getLeaderboardData.mockResolvedValue({
      users: [],
      pagination: {
        page: 1,
        limit: 50,
        totalUsers: 0,
        totalPages: 0,
        hasNext: false,
        hasPrev: false,
      },
      stats: {
        totalTokens: 0,
        totalCost: 0,
        totalActiveTimeMs: 0,
        totalSubmissions: 0,
        uniqueUsers: 0,
      },
      period: "all",
      sortBy: "tokens",
    });

    const impossibleRangeResponse = await GET(
      new Request("http://localhost:3000/api/leaderboard?period=custom&from=2026-02-31&to=2026-03-01&search=alice")
    );
    expect(impossibleRangeResponse.status).toBe(200);
    expect(getLeaderboardData).toHaveBeenCalledWith(
      "all",
      1,
      50,
      "tokens",
      "alice",
      undefined,
      undefined,
    );

    getLeaderboardData.mockClear();

    const invertedRangeResponse = await GET(
      new Request("http://localhost:3000/api/leaderboard?period=custom&from=2026-02-28&to=2026-02-10")
    );
    expect(invertedRangeResponse.status).toBe(200);
    expect(getLeaderboardData).toHaveBeenCalledWith(
      "all",
      1,
      50,
      "tokens",
      "",
      undefined,
      undefined,
    );
  });
});
