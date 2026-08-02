import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

const mockState = vi.hoisted(() => {
  const getSessionFromRequest = vi.fn();
  const getGroupBySlug = vi.fn();
  const getGroupMembership = vi.fn();
  const revalidateGroupCaches = vi.fn();
  const eq = vi.fn((left: unknown, right: unknown) => ({ kind: "eq", left, right }));
  const ne = vi.fn((left: unknown, right: unknown) => ({ kind: "ne", left, right }));
  const and = vi.fn((...conditions: unknown[]) => ({ kind: "and", conditions }));

  let selectRows: Array<Array<Record<string, unknown>>> = [];
  let selectCall = 0;
  let updatedRows: Array<Record<string, unknown>> = [];

  const forUpdate = vi.fn(async () => selectRows[selectCall++] ?? []);
  const limit = vi.fn(() => ({ for: forUpdate }));
  const selectWhere = vi.fn((condition?: unknown) => {
    void condition;
    return { limit, for: forUpdate };
  });
  const from = vi.fn(() => ({ where: selectWhere }));
  const select = vi.fn(() => ({ from }));

  const returning = vi.fn(async () => updatedRows);
  const updateWhere = vi.fn(() => ({ returning }));
  const set = vi.fn(() => ({ where: updateWhere }));
  const update = vi.fn(() => ({ set }));

  const tx = {
    select,
    update,
  };

  const db = {
    transaction: vi.fn(async (cb: (tx: unknown) => Promise<unknown>) => cb(tx)),
    select,
    update,
  };

  return {
    getSessionFromRequest,
    getGroupBySlug,
    getGroupMembership,
    revalidateGroupCaches,
    eq,
    ne,
    and,
    db,
    tx,
    forUpdate,
    selectWhere,
    update,
    set,
    returning,
    reset() {
      getSessionFromRequest.mockReset();
      getGroupBySlug.mockReset();
      getGroupMembership.mockReset();
      revalidateGroupCaches.mockReset();
      eq.mockClear();
      ne.mockClear();
      and.mockClear();
      db.transaction.mockClear();
      select.mockClear();
      from.mockClear();
      selectWhere.mockClear();
      limit.mockClear();
      forUpdate.mockClear();
      update.mockClear();
      set.mockClear();
      updateWhere.mockClear();
      returning.mockClear();
      selectRows = [];
      selectCall = 0;
      updatedRows = [];
    },
    setLockedRows(...rows: Array<Array<Record<string, unknown>>>) {
      selectRows = rows;
      selectCall = 0;
    },
    setUpdatedRows(rows: Array<Record<string, unknown>>) {
      updatedRows = rows;
    },
  };
});

vi.mock("drizzle-orm", () => ({
  and: mockState.and,
  eq: mockState.eq,
  ne: mockState.ne,
}));

vi.mock("@/lib/db", () => ({
  db: mockState.db,
  groupMembers: {
    id: "groupMembers.id",
    groupId: "groupMembers.groupId",
    userId: "groupMembers.userId",
    role: "groupMembers.role",
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

vi.mock("../../src/lib/db/schema", () => ({
  groupRoles: ["owner", "admin", "member"],
}));

type ModuleExports = typeof import("../../src/app/api/groups/[slug]/members/[userId]/role/route");

let PATCH: ModuleExports["PATCH"];

beforeAll(async () => {
  const routeModule = await import("../../src/app/api/groups/[slug]/members/[userId]/role/route");
  PATCH = routeModule.PATCH;
});

beforeEach(() => {
  mockState.reset();
  mockState.getSessionFromRequest.mockResolvedValue({
    id: "actor-1",
    username: "actor",
    displayName: null,
    avatarUrl: null,
  });
  mockState.getGroupBySlug.mockResolvedValue({
    id: "group-1",
    slug: "team",
    name: "Team",
    isPublic: true,
  });
  mockState.getGroupMembership
    .mockResolvedValueOnce({ role: "owner" })
    .mockResolvedValueOnce({ role: "member" });
  mockState.setUpdatedRows([{ id: "membership-1", userId: "target-1", role: "admin" }]);
});

function roleRequest(role: string) {
  return new Request("http://localhost:3000/api/groups/team/members/target-1/role", {
    method: "PATCH",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ role }),
  });
}

function selectLockedUserId(callIndex: number) {
  const condition = mockState.selectWhere.mock.calls[callIndex]?.[0] as
    | {
        conditions?: Array<{ kind?: string; left?: string; right?: string }>;
      }
    | undefined;
  return condition?.conditions?.find(
    (item) => item.kind === "eq" && item.left === "groupMembers.userId"
  )?.right;
}

describe("PATCH /api/groups/[slug]/members/[userId]/role", () => {
  it("rechecks the actor role inside the role update transaction", async () => {
    mockState.setLockedRows(
      [{ userId: "actor-1", role: "member" }],
      [{ userId: "target-1", role: "member" }]
    );

    const response = await PATCH(roleRequest("admin"), {
      params: Promise.resolve({ slug: "team", userId: "target-1" }),
    });

    expect(response.status).toBe(403);
    expect(await response.json()).toEqual({ error: "Forbidden" });
    expect(mockState.db.transaction).toHaveBeenCalledTimes(1);
    expect(mockState.tx.update).not.toHaveBeenCalled();
    expect(mockState.revalidateGroupCaches).not.toHaveBeenCalled();
  });

  it("does not update a target who became an owner before mutation", async () => {
    mockState.setLockedRows(
      [{ userId: "actor-1", role: "owner" }],
      [{ userId: "target-1", role: "owner" }],
      [{ id: "other-owner" }]
    );

    const response = await PATCH(roleRequest("member"), {
      params: Promise.resolve({ slug: "team", userId: "target-1" }),
    });

    expect(response.status).toBe(403);
    expect(await response.json()).toEqual({ error: "Forbidden" });
    expect(mockState.db.transaction).toHaveBeenCalledTimes(1);
    expect(mockState.tx.update).not.toHaveBeenCalled();
    expect(mockState.revalidateGroupCaches).not.toHaveBeenCalled();
  });

  it("rejects demoting the last owner inside the mutation boundary", async () => {
    mockState.getGroupMembership.mockReset();
    mockState.getGroupMembership
      .mockResolvedValueOnce({ role: "owner" })
      .mockResolvedValueOnce({ role: "owner" });
    mockState.setLockedRows(
      [{ userId: "actor-1", role: "owner" }],
      [{ userId: "target-1", role: "owner" }],
      []
    );

    const response = await PATCH(roleRequest("member"), {
      params: Promise.resolve({ slug: "team", userId: "target-1" }),
    });

    expect(response.status).toBe(400);
    expect(await response.json()).toEqual({
      error:
        "Cannot demote the last owner. Use POST /api/groups/:slug/transfer-ownership to assign a new owner first.",
    });
    expect(mockState.db.transaction).toHaveBeenCalledTimes(1);
    expect(mockState.tx.update).not.toHaveBeenCalled();
    expect(mockState.revalidateGroupCaches).not.toHaveBeenCalled();
  });

  it("updates a weaker target only after locking current actor and target roles", async () => {
    mockState.setLockedRows(
      [{ userId: "actor-1", role: "owner" }],
      [{ userId: "target-1", role: "member" }]
    );

    const response = await PATCH(roleRequest("admin"), {
      params: Promise.resolve({ slug: "team", userId: "target-1" }),
    });

    expect(response.status).toBe(200);
    expect(await response.json()).toEqual({ id: "membership-1", userId: "target-1", role: "admin" });
    expect(mockState.db.transaction).toHaveBeenCalledTimes(1);
    expect(mockState.tx.update).toHaveBeenCalledTimes(1);
    expect(mockState.revalidateGroupCaches).toHaveBeenCalledWith("group-1", "team");
  });

  it("locks actor and target memberships in deterministic user id order", async () => {
    mockState.getSessionFromRequest.mockResolvedValue({
      id: "z-actor",
      username: "actor",
      displayName: null,
      avatarUrl: null,
    });
    mockState.setLockedRows([{ role: "member" }], [{ role: "owner" }]);
    mockState.setUpdatedRows([{ id: "membership-1", userId: "a-target", role: "admin" }]);

    const response = await PATCH(roleRequest("admin"), {
      params: Promise.resolve({ slug: "team", userId: "a-target" }),
    });

    expect(response.status).toBe(200);
    expect(selectLockedUserId(0)).toBe("a-target");
    expect(selectLockedUserId(1)).toBe("z-actor");
    expect(await response.json()).toEqual({ id: "membership-1", userId: "a-target", role: "admin" });
    expect(mockState.tx.update).toHaveBeenCalledTimes(1);
  });
});
