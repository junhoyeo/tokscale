import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

const mockState = vi.hoisted(() => {
  const getSessionFromRequest = vi.fn();
  const getGroupBySlug = vi.fn();
  const getGroupMembership = vi.fn();
  const revalidateGroupCaches = vi.fn();
  const eq = vi.fn((left: unknown, right: unknown) => ({ kind: "eq", left, right }));
  const and = vi.fn((...conditions: unknown[]) => ({ kind: "and", conditions }));

  const where = vi.fn(async () => undefined);
  const db = {
    delete: vi.fn(() => ({ where })),
  };

  return {
    getSessionFromRequest,
    getGroupBySlug,
    getGroupMembership,
    revalidateGroupCaches,
    eq,
    and,
    db,
    where,
    reset() {
      getSessionFromRequest.mockReset();
      getGroupBySlug.mockReset();
      getGroupMembership.mockReset();
      revalidateGroupCaches.mockReset();
      eq.mockClear();
      and.mockClear();
      db.delete.mockClear();
      where.mockClear();
    },
  };
});

vi.mock("drizzle-orm", () => ({
  and: mockState.and,
  eq: mockState.eq,
}));

vi.mock("@/lib/db", () => ({
  db: mockState.db,
  groupMembers: {
    groupId: "groupMembers.groupId",
    userId: "groupMembers.userId",
  },
}));

vi.mock("@/lib/auth/requestSession", () => ({
  getSessionFromRequest: mockState.getSessionFromRequest,
}));

vi.mock("@/lib/groups/cache", () => ({
  revalidateGroupCaches: mockState.revalidateGroupCaches,
}));

vi.mock("@/lib/groups/permissions", () => ({
  getGroupMembership: mockState.getGroupMembership,
}));

vi.mock("@/lib/groups/queries", () => ({
  getGroupBySlug: mockState.getGroupBySlug,
}));

type ModuleExports = typeof import("../../src/app/api/groups/[slug]/leave/route");

let POST: ModuleExports["POST"];

beforeAll(async () => {
  const routeModule = await import("../../src/app/api/groups/[slug]/leave/route");
  POST = routeModule.POST;
});

beforeEach(() => {
  mockState.reset();
});

describe("POST /api/groups/[slug]/leave", () => {
  it("returns success when cache invalidation fails after deleting membership", async () => {
    mockState.getSessionFromRequest.mockResolvedValue({
      id: "user-1",
      username: "alice",
      displayName: null,
      avatarUrl: null,
    });
    mockState.getGroupBySlug.mockResolvedValue({
      id: "group-1",
      slug: "team",
      name: "Team",
      isPublic: false,
    });
    mockState.getGroupMembership.mockResolvedValue({ role: "member" });
    mockState.revalidateGroupCaches.mockRejectedValue(new Error("cache unavailable"));

    const response = await POST(
      new Request("http://localhost:3000/api/groups/team/leave", { method: "POST" }),
      { params: Promise.resolve({ slug: "team" }) }
    );

    expect(response.status).toBe(200);
    expect(await response.json()).toEqual({ success: true });
    expect(mockState.db.delete).toHaveBeenCalledTimes(1);
    expect(mockState.revalidateGroupCaches).toHaveBeenCalledWith("group-1", "team");
  });
});
