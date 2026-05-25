import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

const mockState = vi.hoisted(() => {
  const getSessionFromRequest = vi.fn();
  const getGroupBySlug = vi.fn();
  const getGroupMembership = vi.fn();
  const getGroupMemberCount = vi.fn();
  const generateUniqueGroupSlug = vi.fn();
  const revalidateGroupCaches = vi.fn();
  const revalidatePath = vi.fn();
  const eq = vi.fn((left: unknown, right: unknown) => ({ left, right }));

  let updatedRows: Array<Record<string, unknown>> = [];
  const returning = vi.fn(async () => updatedRows);
  const where = vi.fn(() => ({ returning }));
  const set = vi.fn(() => ({ where }));

  const db = {
    update: vi.fn(() => ({ set })),
    delete: vi.fn(),
  };

  return {
    getSessionFromRequest,
    getGroupBySlug,
    getGroupMembership,
    getGroupMemberCount,
    generateUniqueGroupSlug,
    revalidateGroupCaches,
    revalidatePath,
    eq,
    db,
    set,
    where,
    returning,
    reset() {
      getSessionFromRequest.mockReset();
      getGroupBySlug.mockReset();
      getGroupMembership.mockReset();
      getGroupMemberCount.mockReset();
      generateUniqueGroupSlug.mockReset();
      revalidateGroupCaches.mockReset();
      revalidatePath.mockReset();
      eq.mockClear();
      db.update.mockClear();
      db.delete.mockClear();
      set.mockClear();
      where.mockClear();
      returning.mockClear();
      updatedRows = [];
    },
    setUpdatedRows(rows: Array<Record<string, unknown>>) {
      updatedRows = rows;
    },
  };
});

vi.mock("next/cache", () => ({
  revalidatePath: mockState.revalidatePath,
}));

vi.mock("drizzle-orm", () => ({
  eq: mockState.eq,
}));

vi.mock("@/lib/db", () => ({
  db: mockState.db,
  groups: {
    id: "groups.id",
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
  getGroupMemberCount: mockState.getGroupMemberCount,
}));

vi.mock("@/lib/groups/slugs", () => ({
  generateUniqueGroupSlug: mockState.generateUniqueGroupSlug,
}));

type ModuleExports = typeof import("../../src/app/api/groups/[slug]/route");

let GET: ModuleExports["GET"];
let PATCH: ModuleExports["PATCH"];

beforeAll(async () => {
  const routeModule = await import("../../src/app/api/groups/[slug]/route");
  GET = routeModule.GET;
  PATCH = routeModule.PATCH;
});

beforeEach(() => {
  mockState.reset();
});

function group(overrides: Record<string, unknown> = {}) {
  return {
    id: "group-1",
    name: "Team",
    slug: "team",
    description: "Current description",
    avatarUrl: "https://example.com/avatar.png",
    isPublic: true,
    createdBy: "owner-1",
    createdAt: new Date("2026-01-01T00:00:00Z"),
    updatedAt: new Date("2026-01-01T00:00:00Z"),
    ...overrides,
  };
}

describe("/api/groups/[slug]", () => {
  it("returns the same 404 response for missing groups and private groups without membership", async () => {
    mockState.getGroupBySlug.mockResolvedValueOnce(null);

    const missingResponse = await GET(
      new Request("http://localhost:3000/api/groups/missing"),
      { params: Promise.resolve({ slug: "missing" }) }
    );

    mockState.getGroupBySlug.mockResolvedValueOnce(group({ isPublic: false }));
    mockState.getSessionFromRequest.mockResolvedValueOnce(null);

    const privateResponse = await GET(
      new Request("http://localhost:3000/api/groups/team"),
      { params: Promise.resolve({ slug: "team" }) }
    );

    expect(missingResponse!.status).toBe(404);
    expect(privateResponse!.status).toBe(404);
    expect(await missingResponse!.json()).toEqual(await privateResponse!.json());
    expect(mockState.getGroupMemberCount).not.toHaveBeenCalled();
  });

  it("rejects non-string nullable PATCH fields without updating the group", async () => {
    mockState.getSessionFromRequest.mockResolvedValue({
      id: "admin-1",
      username: "admin",
      displayName: null,
      avatarUrl: null,
    });
    mockState.getGroupBySlug.mockResolvedValue(group());
    mockState.getGroupMembership.mockResolvedValue({ role: "admin" });

    const response = await PATCH(
      new Request("http://localhost:3000/api/groups/team", {
        method: "PATCH",
        body: JSON.stringify({ description: 123 }),
      }),
      { params: Promise.resolve({ slug: "team" }) }
    );

    expect(response.status).toBe(400);
    expect(await response.json()).toEqual({
      error: "description must be a string or null",
    });
    expect(mockState.db.update).not.toHaveBeenCalled();
  });

  it("allows explicit null PATCH fields to clear optional group metadata", async () => {
    mockState.getSessionFromRequest.mockResolvedValue({
      id: "admin-1",
      username: "admin",
      displayName: null,
      avatarUrl: null,
    });
    mockState.getGroupBySlug.mockResolvedValue(group());
    mockState.getGroupMembership.mockResolvedValue({ role: "admin" });
    mockState.setUpdatedRows([
      group({
        description: null,
        avatarUrl: null,
      }),
    ]);

    const response = await PATCH(
      new Request("http://localhost:3000/api/groups/team", {
        method: "PATCH",
        body: JSON.stringify({ description: null, avatarUrl: null }),
      }),
      { params: Promise.resolve({ slug: "team" }) }
    );

    expect(response.status).toBe(200);
    expect(mockState.set).toHaveBeenCalledWith(
      expect.objectContaining({
        description: null,
        avatarUrl: null,
      })
    );
    expect(mockState.revalidateGroupCaches).toHaveBeenCalledWith("group-1", "team");
  });
});
