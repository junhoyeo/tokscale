import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

const mockState = vi.hoisted(() => {
  const getSessionFromRequest = vi.fn();
  const getGroupBySlug = vi.fn();
  const getGroupMembership = vi.fn();
  const canManageGroupMember = vi.fn();
  const revalidateGroupCaches = vi.fn();
  const eq = vi.fn((left: unknown, right: unknown) => ({ kind: "eq", left, right }));
  const and = vi.fn((...conditions: unknown[]) => ({ kind: "and", conditions }));

  let selectRows: Array<Array<Record<string, unknown>>> = [];
  let selectCall = 0;
  let deletedRows: Array<Record<string, unknown>> = [];

  const forUpdate = vi.fn(async () => selectRows[selectCall++] ?? []);
  const limit = vi.fn(() => ({ for: forUpdate }));
  const selectWhere = vi.fn(() => ({ limit }));
  const from = vi.fn(() => ({ where: selectWhere }));
  const select = vi.fn(() => ({ from }));

  const returning = vi.fn(async () => deletedRows);
  const deleteWhere = vi.fn(() => ({ returning }));
  const deleteBuilder = vi.fn(() => ({ where: deleteWhere }));

  const tx = {
    select,
    delete: deleteBuilder,
  };

  const db = {
    transaction: vi.fn(async (cb: (tx: unknown) => Promise<unknown>) => cb(tx)),
    delete: vi.fn(() => ({ where: deleteWhere })),
  };

  return {
    getSessionFromRequest,
    getGroupBySlug,
    getGroupMembership,
    canManageGroupMember,
    revalidateGroupCaches,
    eq,
    and,
    db,
    tx,
    forUpdate,
    returning,
    deleteBuilder,
    reset() {
      getSessionFromRequest.mockReset();
      getGroupBySlug.mockReset();
      getGroupMembership.mockReset();
      canManageGroupMember.mockReset();
      revalidateGroupCaches.mockReset();
      eq.mockClear();
      and.mockClear();
      db.transaction.mockClear();
      db.delete.mockClear();
      select.mockClear();
      from.mockClear();
      selectWhere.mockClear();
      limit.mockClear();
      forUpdate.mockClear();
      deleteBuilder.mockClear();
      deleteWhere.mockClear();
      returning.mockClear();
      selectRows = [];
      selectCall = 0;
      deletedRows = [];
    },
    setLockedRows(actorRows: Array<Record<string, unknown>>, targetRows: Array<Record<string, unknown>>) {
      selectRows = [actorRows, targetRows];
      selectCall = 0;
    },
    setDeletedRows(rows: Array<Record<string, unknown>>) {
      deletedRows = rows;
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
    id: "groupMembers.id",
    groupId: "groupMembers.groupId",
    userId: "groupMembers.userId",
    role: "groupMembers.role",
  },
  users: {
    id: "users.id",
    username: "users.username",
    displayName: "users.displayName",
    avatarUrl: "users.avatarUrl",
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
  canManageGroupMember: mockState.canManageGroupMember,
}));

vi.mock("@/lib/groups/queries", () => ({
  getGroupBySlug: mockState.getGroupBySlug,
}));

vi.mock("../../src/lib/db/schema", () => ({
  groupRoles: ["owner", "admin", "member"],
}));

type ModuleExports = typeof import("../../src/app/api/groups/[slug]/members/route");

let DELETE: ModuleExports["DELETE"];

beforeAll(async () => {
  const routeModule = await import("../../src/app/api/groups/[slug]/members/route");
  DELETE = routeModule.DELETE;
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
  mockState.getGroupMembership.mockResolvedValue({ role: "owner" });
  mockState.canManageGroupMember.mockResolvedValue(true);
  mockState.setDeletedRows([{ id: "membership-1" }]);
});

function deleteRequest(userId: string) {
  return new Request(`http://localhost:3000/api/groups/team/members?userId=${userId}`, {
    method: "DELETE",
  });
}

describe("DELETE /api/groups/[slug]/members", () => {
  it("rechecks the actor role inside the delete transaction", async () => {
    mockState.setLockedRows(
      [{ userId: "actor-1", role: "member" }],
      [{ userId: "target-1", role: "member" }]
    );

    const response = await DELETE(deleteRequest("target-1"), {
      params: Promise.resolve({ slug: "team" }),
    });

    expect(response.status).toBe(403);
    expect(await response.json()).toEqual({ error: "Forbidden" });
    expect(mockState.db.transaction).toHaveBeenCalledTimes(1);
    expect(mockState.tx.delete).not.toHaveBeenCalled();
    expect(mockState.revalidateGroupCaches).not.toHaveBeenCalled();
  });

  it("does not delete a member who became an owner before mutation", async () => {
    mockState.setLockedRows(
      [{ userId: "actor-1", role: "owner" }],
      [{ userId: "target-1", role: "owner" }]
    );

    const response = await DELETE(deleteRequest("target-1"), {
      params: Promise.resolve({ slug: "team" }),
    });

    expect(response.status).toBe(403);
    expect(await response.json()).toEqual({ error: "Forbidden" });
    expect(mockState.db.transaction).toHaveBeenCalledTimes(1);
    expect(mockState.tx.delete).not.toHaveBeenCalled();
    expect(mockState.revalidateGroupCaches).not.toHaveBeenCalled();
  });
});
