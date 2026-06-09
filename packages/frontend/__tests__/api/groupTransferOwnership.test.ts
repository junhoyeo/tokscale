import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

/**
 * B4: POST /api/groups/[slug]/transfer-ownership
 */

const mockState = vi.hoisted(() => {
  const getSessionFromRequest = vi.fn();
  const getGroupBySlug = vi.fn();
  const getGroupMembership = vi.fn();
  const revalidateGroupCaches = vi.fn();
  const eq = vi.fn((left: unknown, right: unknown) => ({ kind: "eq", left, right }));
  const ne = vi.fn((left: unknown, right: unknown) => ({ kind: "ne", left, right }));
  const and = vi.fn((...c: unknown[]) => ({ kind: "and", c }));

  let txUpdateRows: Array<Array<Record<string, unknown>>> = [];
  let callIndex = 0;

  const returning = vi.fn(async () => txUpdateRows[callIndex++] ?? []);
  const where = vi.fn(() => ({ returning }));
  const set = vi.fn(() => ({ where }));

  const tx = {
    update: vi.fn(() => ({ set })),
  };

  const db = {
    transaction: vi.fn(async (cb: (tx: unknown) => Promise<unknown>) => cb(tx)),
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
    set,
    where,
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
      tx.update.mockClear();
      set.mockClear();
      where.mockClear();
      returning.mockClear();
      txUpdateRows = [];
      callIndex = 0;
    },
    setTransactionRows(callerRow: Record<string, unknown>, targetRow: Record<string, unknown>) {
      txUpdateRows = [[callerRow], [targetRow]];
      callIndex = 0;
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

type ModuleExports =
  typeof import("../../src/app/api/groups/[slug]/transfer-ownership/route");

let POST: ModuleExports["POST"];

beforeAll(async () => {
  const routeModule = await import(
    "../../src/app/api/groups/[slug]/transfer-ownership/route"
  );
  POST = routeModule.POST;
});

beforeEach(() => mockState.reset());

function group() {
  return {
    id: "group-1",
    slug: "team",
    name: "Team",
    isPublic: true,
    createdBy: "owner-1",
    description: null,
    avatarUrl: null,
    createdAt: new Date().toISOString(),
    updatedAt: new Date().toISOString(),
  };
}

function ownerSession() {
  return { id: "owner-1", username: "alice", displayName: null, avatarUrl: null };
}

function mockBrowserSessionOnly() {
  mockState.getSessionFromRequest.mockImplementation(
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

      return ownerSession();
    }
  );
}

function makeRequest(body: unknown) {
  return new Request("http://localhost:3000/api/groups/team/transfer-ownership", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
}

describe("POST /api/groups/[slug]/transfer-ownership", () => {
  it("rejects Authorization header sessions when transferring ownership", async () => {
    mockBrowserSessionOnly();
    mockState.getGroupBySlug.mockResolvedValue(group());
    mockState.getGroupMembership.mockResolvedValue({ role: "owner" });

    const response = await POST(
      new Request("http://localhost:3000/api/groups/team/transfer-ownership", {
        method: "POST",
        headers: {
          Authorization: "Bearer tt_personal",
          "Content-Type": "application/json",
          Origin: "http://localhost:3000",
        },
        body: JSON.stringify({ targetUserId: "bob" }),
      }),
      { params: Promise.resolve({ slug: "team" }) }
    );

    expect(response.status).toBe(401);
    expect(await response.json()).toEqual({ error: "Not authenticated" });
    expect(mockState.getGroupBySlug).not.toHaveBeenCalled();
    expect(mockState.db.transaction).not.toHaveBeenCalled();
  });

  it("returns 401 when not authenticated", async () => {
    mockState.getSessionFromRequest.mockResolvedValue(null);
    mockState.getGroupBySlug.mockResolvedValue(group());

    const response = await POST(makeRequest({ targetUserId: "bob" }), {
      params: Promise.resolve({ slug: "team" }),
    });

    expect(response.status).toBe(401);
  });

  it("returns 403 when caller is not the owner", async () => {
    mockState.getSessionFromRequest.mockResolvedValue({
      id: "admin-1",
      username: "bob",
      displayName: null,
      avatarUrl: null,
    });
    mockState.getGroupBySlug.mockResolvedValue(group());
    mockState.getGroupMembership.mockResolvedValue({ role: "admin" });

    const response = await POST(makeRequest({ targetUserId: "carol" }), {
      params: Promise.resolve({ slug: "team" }),
    });

    expect(response.status).toBe(403);
    expect(await response.json()).toMatchObject({ error: expect.stringContaining("owner") });
    expect(mockState.db.transaction).not.toHaveBeenCalled();
  });

  it("returns 400 when target is not a member of the group", async () => {
    mockState.getSessionFromRequest.mockResolvedValue(ownerSession());
    mockState.getGroupBySlug.mockResolvedValue(group());
    // First call: caller membership (owner), second call: target membership (null)
    mockState.getGroupMembership
      .mockResolvedValueOnce({ role: "owner" })
      .mockResolvedValueOnce(null);

    const response = await POST(makeRequest({ targetUserId: "non-member" }), {
      params: Promise.resolve({ slug: "team" }),
    });

    expect(response.status).toBe(400);
    expect(await response.json()).toMatchObject({
      error: expect.stringContaining("not a member"),
    });
    expect(mockState.db.transaction).not.toHaveBeenCalled();
  });

  it("returns 400 when targetUserId is missing from the body", async () => {
    mockState.getSessionFromRequest.mockResolvedValue(ownerSession());
    mockState.getGroupBySlug.mockResolvedValue(group());
    mockState.getGroupMembership.mockResolvedValue({ role: "owner" });

    const response = await POST(makeRequest({}), {
      params: Promise.resolve({ slug: "team" }),
    });

    expect(response.status).toBe(400);
    expect(await response.json()).toMatchObject({ error: expect.stringContaining("targetUserId") });
  });

  it("returns 400 for non-object body shapes", async () => {
    mockState.getSessionFromRequest.mockResolvedValue(ownerSession());
    mockState.getGroupBySlug.mockResolvedValue(group());
    mockState.getGroupMembership.mockResolvedValue({ role: "owner" });

    for (const rawBody of ["null", "[]", '"string"']) {
      mockState.reset();
      mockState.getSessionFromRequest.mockResolvedValue(ownerSession());
      mockState.getGroupBySlug.mockResolvedValue(group());
      mockState.getGroupMembership.mockResolvedValue({ role: "owner" });

      const response = await POST(
        new Request("http://localhost:3000/api/groups/team/transfer-ownership", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: rawBody,
        }),
        { params: Promise.resolve({ slug: "team" }) }
      );

      expect(response.status).toBe(400);
      expect(await response.json()).toEqual({ error: "Invalid JSON body" });
    }
  });

  it("atomically transfers ownership: caller becomes admin, target becomes owner", async () => {
    mockState.getSessionFromRequest.mockResolvedValue(ownerSession());
    mockState.getGroupBySlug.mockResolvedValue(group());
    mockState.getGroupMembership
      .mockResolvedValueOnce({ role: "owner" })   // caller
      .mockResolvedValueOnce({ role: "admin" });  // target

    mockState.setTransactionRows(
      { id: "m1", userId: "owner-1", role: "admin" },
      { id: "m2", userId: "bob", role: "owner" }
    );

    const response = await POST(makeRequest({ targetUserId: "bob" }), {
      params: Promise.resolve({ slug: "team" }),
    });

    expect(response.status).toBe(200);
    const body = await response.json();
    expect(body.members).toHaveLength(2);
    expect(body.members[0]).toMatchObject({ userId: "owner-1", role: "admin" });
    expect(body.members[1]).toMatchObject({ userId: "bob", role: "owner" });

    // Two updates must happen inside the same transaction
    expect(mockState.db.transaction).toHaveBeenCalledTimes(1);
    expect(mockState.tx.update).toHaveBeenCalledTimes(2);
    expect(mockState.revalidateGroupCaches).toHaveBeenCalledWith("group-1", "team");
  });

  it("returns 409 when the predicate-guarded caller update finds zero rows (race lost)", async () => {
    mockState.getSessionFromRequest.mockResolvedValue(ownerSession());
    mockState.getGroupBySlug.mockResolvedValue(group());
    mockState.getGroupMembership
      .mockResolvedValueOnce({ role: "owner" })
      .mockResolvedValueOnce({ role: "admin" });

    // Simulate a concurrent transfer demoting the caller before this tx commits:
    // the role='owner' predicate in the caller UPDATE matches zero rows.
    mockState.setTransactionRows({}, { id: "m2", userId: "bob", role: "owner" });
    // Override callerRow specifically to be undefined (empty array)
    (mockState as unknown as { setTransactionRows: (a: unknown, b: unknown) => void });
    // Re-set txUpdateRows so the FIRST returning() call yields []
    (mockState as unknown as { reset: () => void }).reset();
    mockState.getSessionFromRequest.mockResolvedValue(ownerSession());
    mockState.getGroupBySlug.mockResolvedValue(group());
    mockState.getGroupMembership
      .mockResolvedValueOnce({ role: "owner" })
      .mockResolvedValueOnce({ role: "admin" });
    // Wire returning() to return [] on the first call (caller race-lost)
    let call = 0;
    mockState.returning.mockImplementation(async () => (call++ === 0 ? [] : [{ id: "m2", userId: "bob", role: "owner" }]));

    const response = await POST(makeRequest({ targetUserId: "bob" }), {
      params: Promise.resolve({ slug: "team" }),
    });

    expect(response.status).toBe(409);
    const body = await response.json();
    expect(body.error).toMatch(/changed|retry/i);
    expect(mockState.revalidateGroupCaches).not.toHaveBeenCalled();
  });

  it("returns 409 when the target became an owner before promotion (race lost)", async () => {
    mockState.getSessionFromRequest.mockResolvedValue(ownerSession());
    mockState.getGroupBySlug.mockResolvedValue(group());
    mockState.getGroupMembership
      .mockResolvedValueOnce({ role: "owner" })
      .mockResolvedValueOnce({ role: "admin" });

    // Caller demote succeeds, but target promotion finds zero rows because
    // role != 'owner' predicate doesn't match (target was promoted concurrently).
    let call = 0;
    mockState.returning.mockImplementation(async () =>
      call++ === 0 ? [{ id: "m1", userId: "owner-1", role: "admin" }] : []
    );

    const response = await POST(makeRequest({ targetUserId: "bob" }), {
      params: Promise.resolve({ slug: "team" }),
    });

    expect(response.status).toBe(409);
    expect(mockState.revalidateGroupCaches).not.toHaveBeenCalled();
  });
});
