import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

/**
 * B2: Payload guard tests for three mutating routes:
 * - POST /api/groups (create group)
 * - PATCH /api/groups/[slug] (update group)
 * - PATCH /api/groups/[slug]/members/[userId]/role (update member role)
 */

// ─── Shared mock factory ─────────────────────────────────────────────────────

const mockGroups = vi.hoisted(() => {
  const getSessionFromRequest = vi.fn();
  const getGroupBySlug = vi.fn();
  const getGroupMembership = vi.fn();
  const revalidateGroupCaches = vi.fn();
  const revalidatePath = vi.fn();
  const generateUniqueGroupSlug = vi.fn();
  const getGroupMemberCount = vi.fn();
  const eq = vi.fn((left: unknown, right: unknown) => ({ left, right }));
  const and = vi.fn((...c: unknown[]) => ({ and: c }));

  let updatedRows: Array<Record<string, unknown>> = [];
  const returning = vi.fn(async () => updatedRows);
  const where = vi.fn(() => ({ returning }));
  const set = vi.fn(() => ({ where }));
  const insertValues = vi.fn(async () => []);
  const insert = vi.fn(() => ({ values: insertValues }));

  const db = {
    update: vi.fn(() => ({ set })),
    insert,
    transaction: vi.fn(async (cb: (tx: unknown) => Promise<unknown>) =>
      cb({
        insert: vi.fn(() => ({ values: vi.fn(async () => [{ id: "g1", slug: "team" }]) })),
      })
    ),
  };

  return {
    getSessionFromRequest,
    getGroupBySlug,
    getGroupMembership,
    revalidateGroupCaches,
    revalidatePath,
    generateUniqueGroupSlug,
    getGroupMemberCount,
    eq,
    and,
    db,
    set,
    where,
    returning,
    reset() {
      getSessionFromRequest.mockReset();
      getGroupBySlug.mockReset();
      getGroupMembership.mockReset();
      revalidateGroupCaches.mockReset();
      revalidatePath.mockReset();
      generateUniqueGroupSlug.mockReset();
      getGroupMemberCount.mockReset();
      eq.mockClear();
      and.mockClear();
      db.update.mockClear();
      db.insert.mockClear();
      db.transaction.mockClear();
      set.mockClear();
      where.mockClear();
      returning.mockClear();
      updatedRows = [];
    },
  };
});

vi.mock("next/cache", () => ({ revalidatePath: mockGroups.revalidatePath }));

vi.mock("drizzle-orm", () => ({
  eq: mockGroups.eq,
  and: mockGroups.and,
  ne: vi.fn(),
  sql: vi.fn(),
}));

vi.mock("@/lib/db", () => ({
  db: mockGroups.db,
  groups: { id: "groups.id" },
  groupMembers: {
    id: "groupMembers.id",
    groupId: "groupMembers.groupId",
    userId: "groupMembers.userId",
    role: "groupMembers.role",
  },
}));

vi.mock("@/lib/auth/requestSession", () => ({
  getSessionFromRequest: mockGroups.getSessionFromRequest,
}));

vi.mock("@/lib/groups/cache", () => ({
  revalidateGroupCaches: mockGroups.revalidateGroupCaches,
}));

vi.mock("@/lib/groups/permissions", () => ({
  getGroupMembership: mockGroups.getGroupMembership,
}));

vi.mock("@/lib/groups/queries", () => ({
  getGroupBySlug: mockGroups.getGroupBySlug,
  getGroupMemberCount: mockGroups.getGroupMemberCount,
}));

vi.mock("@/lib/groups/slugs", () => ({
  generateUniqueGroupSlug: mockGroups.generateUniqueGroupSlug,
}));

vi.mock("@/lib/groups/utils", () => ({
  isGroupRole: (r: unknown) => r === "member" || r === "admin" || r === "owner",
  canManageGroupRole: (actor: string, target: string) => {
    const level: Record<string, number> = { owner: 3, admin: 2, member: 1 };
    return level[actor] > level[target];
  },
}));

// ─── Route module imports (lazy, after mocks) ────────────────────────────────

type GroupsRoute = typeof import("../../src/app/api/groups/route");
type SlugRoute = typeof import("../../src/app/api/groups/[slug]/route");
type RoleRoute = typeof import("../../src/app/api/groups/[slug]/members/[userId]/role/route");

let groupsPOST: GroupsRoute["POST"];
let slugPATCH: SlugRoute["PATCH"];
let rolePATCH: RoleRoute["PATCH"];

beforeAll(async () => {
  groupsPOST = (await import("../../src/app/api/groups/route")).POST;
  slugPATCH = (await import("../../src/app/api/groups/[slug]/route")).PATCH;
  rolePATCH = (
    await import("../../src/app/api/groups/[slug]/members/[userId]/role/route")
  ).PATCH;
});

beforeEach(() => mockGroups.reset());

// ─── Helpers ─────────────────────────────────────────────────────────────────

function session() {
  return { id: "user-1", username: "alice", displayName: null, avatarUrl: null };
}

function mockBrowserSessionOnly() {
  mockGroups.getSessionFromRequest.mockImplementation(
    async (
      request: Request,
      options?: { allowAuthorizationHeader?: boolean }
    ) => {
      if (
        request.headers.has("Authorization") &&
        options?.allowAuthorizationHeader === false
      ) {
        return null;
      }

      return session();
    }
  );
}

function group() {
  return {
    id: "group-1",
    slug: "team",
    name: "Team",
    isPublic: true,
    createdBy: "user-2",
    description: null,
    avatarUrl: null,
    createdAt: new Date().toISOString(),
    updatedAt: new Date().toISOString(),
  };
}

const BAD_BODIES = [
  { label: "null literal", body: "null" },
  { label: "string literal", body: '"string"' },
  { label: "array literal", body: "[]" },
];

// ─── POST /api/groups ─────────────────────────────────────────────────────────

describe("POST /api/groups — payload guard (B2)", () => {
  it("rejects Authorization header sessions when creating groups", async () => {
    mockBrowserSessionOnly();

    const response = await groupsPOST(
      new Request("http://localhost:3000/api/groups", {
        method: "POST",
        body: JSON.stringify({ name: "Team" }),
        headers: {
          Authorization: "Bearer tt_personal",
          "Content-Type": "application/json",
          Origin: "http://localhost:3000",
        },
      })
    );

    expect(response.status).toBe(401);
    expect(await response.json()).toEqual({ error: "Not authenticated" });
    expect(mockGroups.db.transaction).not.toHaveBeenCalled();
  });

  it.each(BAD_BODIES)("returns 400 for $label body", async ({ body }) => {
    mockGroups.getSessionFromRequest.mockResolvedValue(session());

    const response = await groupsPOST(
      new Request("http://localhost:3000/api/groups", {
        method: "POST",
        body,
        headers: { "Content-Type": "application/json" },
      })
    );

    expect(response.status).toBe(400);
    expect(await response.json()).toEqual({ error: "Invalid JSON body" });
    expect(mockGroups.db.transaction).not.toHaveBeenCalled();
  });
});

// ─── PATCH /api/groups/[slug] ─────────────────────────────────────────────────

describe("PATCH /api/groups/[slug] — payload guard (B2)", () => {
  it("rejects Authorization header sessions when updating groups", async () => {
    mockBrowserSessionOnly();
    mockGroups.getGroupBySlug.mockResolvedValue(group());
    mockGroups.getGroupMembership.mockResolvedValue({ role: "admin" });

    const response = await slugPATCH(
      new Request("http://localhost:3000/api/groups/team", {
        method: "PATCH",
        body: JSON.stringify({ description: "Updated" }),
        headers: {
          Authorization: "Bearer tt_personal",
          "Content-Type": "application/json",
          Origin: "http://localhost:3000",
        },
      }),
      { params: Promise.resolve({ slug: "team" }) }
    );

    expect(response.status).toBe(401);
    expect(await response.json()).toEqual({ error: "Not authenticated" });
    expect(mockGroups.getGroupBySlug).not.toHaveBeenCalled();
    expect(mockGroups.db.update).not.toHaveBeenCalled();
  });

  it.each(BAD_BODIES)("returns 400 for $label body", async ({ body }) => {
    mockGroups.getSessionFromRequest.mockResolvedValue(session());
    mockGroups.getGroupBySlug.mockResolvedValue(group());
    mockGroups.getGroupMembership.mockResolvedValue({ role: "admin" });

    const response = await slugPATCH(
      new Request("http://localhost:3000/api/groups/team", {
        method: "PATCH",
        body,
        headers: { "Content-Type": "application/json" },
      }),
      { params: Promise.resolve({ slug: "team" }) }
    );

    expect(response.status).toBe(400);
    expect(await response.json()).toEqual({ error: "Invalid JSON body" });
    expect(mockGroups.db.update).not.toHaveBeenCalled();
  });
});

// ─── PATCH /api/groups/[slug]/members/[userId]/role ───────────────────────────

describe("PATCH /api/groups/[slug]/members/[userId]/role — payload guard (B2)", () => {
  it("rejects Authorization header sessions when updating member roles", async () => {
    mockBrowserSessionOnly();
    mockGroups.getGroupBySlug.mockResolvedValue(group());
    mockGroups.getGroupMembership.mockResolvedValue({ role: "owner" });

    const response = await rolePATCH(
      new Request("http://localhost:3000/api/groups/team/members/user-2/role", {
        method: "PATCH",
        body: JSON.stringify({ role: "admin" }),
        headers: {
          Authorization: "Bearer tt_personal",
          "Content-Type": "application/json",
          Origin: "http://localhost:3000",
        },
      }),
      { params: Promise.resolve({ slug: "team", userId: "user-2" }) }
    );

    expect(response.status).toBe(401);
    expect(await response.json()).toEqual({ error: "Not authenticated" });
    expect(mockGroups.getGroupBySlug).not.toHaveBeenCalled();
    expect(mockGroups.db.update).not.toHaveBeenCalled();
  });

  it.each(BAD_BODIES)("returns 400 for $label body", async ({ body }) => {
    mockGroups.getSessionFromRequest.mockResolvedValue(session());
    mockGroups.getGroupBySlug.mockResolvedValue(group());

    const response = await rolePATCH(
      new Request("http://localhost:3000/api/groups/team/members/user-2/role", {
        method: "PATCH",
        body,
        headers: { "Content-Type": "application/json" },
      }),
      { params: Promise.resolve({ slug: "team", userId: "user-2" }) }
    );

    expect(response.status).toBe(400);
    expect(await response.json()).toEqual({ error: "Invalid JSON body" });
    expect(mockGroups.db.update).not.toHaveBeenCalled();
  });
});
