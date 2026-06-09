import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

const mockState = vi.hoisted(() => {
  const getSessionFromRequest = vi.fn();
  const getGroupBySlug = vi.fn();
  const getGroupMembership = vi.fn();
  const getGroupLeaderboardData = vi.fn();

  return {
    getSessionFromRequest,
    getGroupBySlug,
    getGroupMembership,
    getGroupLeaderboardData,
    reset() {
      getSessionFromRequest.mockReset();
      getGroupBySlug.mockReset();
      getGroupMembership.mockReset();
      getGroupLeaderboardData.mockReset();
    },
  };
});

vi.mock("@/lib/auth/requestSession", () => ({
  getSessionFromRequest: mockState.getSessionFromRequest,
}));

vi.mock("@/lib/groups/queries", () => ({
  getGroupBySlug: mockState.getGroupBySlug,
}));

vi.mock("@/lib/groups/permissions", () => ({
  getGroupMembership: mockState.getGroupMembership,
}));

vi.mock("@/lib/groups/getGroupLeaderboard", () => ({
  getGroupLeaderboardData: mockState.getGroupLeaderboardData,
}));

type ModuleExports = typeof import("../../src/app/api/groups/[slug]/leaderboard/route");

let GET: ModuleExports["GET"];

beforeAll(async () => {
  const routeModule = await import("../../src/app/api/groups/[slug]/leaderboard/route");
  GET = routeModule.GET;
});

beforeEach(() => {
  mockState.reset();
});

describe("GET /api/groups/[slug]/leaderboard", () => {
  it("hides private group leaderboard data from non-members", async () => {
    mockState.getGroupBySlug.mockResolvedValue({
      id: "group-1",
      slug: "team",
      name: "Team",
      isPublic: false,
    });
    mockState.getSessionFromRequest.mockResolvedValue({
      id: "user-outsider",
      username: "outsider",
      displayName: null,
      avatarUrl: null,
    });
    mockState.getGroupMembership.mockResolvedValue(null);

    const response = await GET(
      new Request("http://localhost:3000/api/groups/team/leaderboard?period=week"),
      { params: Promise.resolve({ slug: "team" }) }
    );

    expect(response.status).toBe(404);
    expect(mockState.getGroupLeaderboardData).not.toHaveBeenCalled();
  });

  it("passes scoped query options to the group leaderboard service for members", async () => {
    mockState.getGroupBySlug.mockResolvedValue({
      id: "group-1",
      slug: "team",
      name: "Team",
      isPublic: false,
    });
    mockState.getSessionFromRequest.mockResolvedValue({
      id: "user-member",
      username: "member",
      displayName: null,
      avatarUrl: null,
    });
    mockState.getGroupMembership.mockResolvedValue({ role: "member" });
    mockState.getGroupLeaderboardData.mockResolvedValue({
      users: [],
      pagination: { page: 2, limit: 25, totalUsers: 0, totalPages: 0, hasNext: false, hasPrev: true },
      stats: { totalTokens: 0, totalCost: 0, activeUsers: 0, totalMembers: 1 },
      period: "month",
      sortBy: "cost",
    });

    const response = await GET(
      new Request("http://localhost:3000/api/groups/team/leaderboard?period=month&page=2&limit=25&sortBy=cost&search=mem"),
      { params: Promise.resolve({ slug: "team" }) }
    );

    expect(response.status).toBe(200);
    expect(mockState.getGroupLeaderboardData).toHaveBeenCalledWith(
      "group-1",
      "month",
      2,
      25,
      "cost",
      "mem"
    );
  });
});
