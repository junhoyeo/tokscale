import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

const mockState = vi.hoisted(() => {
  const getSession = vi.fn();
  const getSessionFromHeader = vi.fn();
  const clearSession = vi.fn();
  const revalidateTag = vi.fn();
  const revalidatePath = vi.fn();
  const revalidateUserGroupLeaderboards = vi.fn();
  const revalidateUsernamePaths = vi.fn((username: string) => {
    const lower = username.toLowerCase();
    const variants = username === lower ? [username] : [username, lower];
    for (const variant of variants) {
      revalidatePath(`/u/${variant}`);
      revalidatePath(`/api/users/${variant}`);
      revalidatePath(`/api/embed/${variant}/svg`);
    }
  });
  const eq = vi.fn((left: unknown, right: unknown) => ({
    kind: "eq",
    left,
    right,
  }));
  const returning = vi.fn(async () => {
    if (deleteError) {
      throw deleteError;
    }
    return deletedRows;
  });
  const where = vi.fn(() => ({
    returning,
  }));
  let deletedRows: Array<{ id: string }> = [];
  let deleteError: Error | null = null;

  const db = {
    delete: vi.fn(() => ({
      where,
    })),
  };

  return {
    getSession,
    getSessionFromHeader,
    clearSession,
    revalidateTag,
    revalidatePath,
    revalidateUserGroupLeaderboards,
    revalidateUsernamePaths,
    eq,
    db,
    where,
    reset() {
      getSession.mockReset();
      getSessionFromHeader.mockReset();
      clearSession.mockReset();
      revalidateTag.mockReset();
      revalidatePath.mockReset();
      revalidateUserGroupLeaderboards.mockReset();
      revalidateUsernamePaths.mockClear();
      eq.mockClear();
      db.delete.mockClear();
      where.mockClear();
      returning.mockClear();
      deletedRows = [];
      deleteError = null;
    },
    setDeletedRows(rows: Array<{ id: string }>) {
      deletedRows = rows;
    },
    setDeleteError(error: Error | null) {
      deleteError = error;
    },
  };
});

vi.mock("next/cache", () => ({
  revalidateTag: mockState.revalidateTag,
  revalidatePath: mockState.revalidatePath,
}));

vi.mock("drizzle-orm", () => ({
  eq: mockState.eq,
}));

vi.mock("@/lib/auth/session", () => ({
  getSession: mockState.getSession,
  getSessionFromHeader: mockState.getSessionFromHeader,
  clearSession: mockState.clearSession,
}));

vi.mock("@/lib/db", () => ({
  db: mockState.db,
  users: {
    id: "users.id",
  },
}));

vi.mock("@/lib/db/usernameLookup", () => ({
  normalizeUsernameCacheKey: (username: string) => username.toLowerCase(),
  revalidateUsernamePaths: mockState.revalidateUsernamePaths,
}));

vi.mock("@/lib/groups/cache", () => ({
  revalidateUserGroupLeaderboards: mockState.revalidateUserGroupLeaderboards,
}));

type ModuleExports = typeof import("../../src/app/api/settings/account/route");

let DELETE: ModuleExports["DELETE"];

beforeAll(async () => {
  const routeModule = await import("../../src/app/api/settings/account/route");
  DELETE = routeModule.DELETE;
});

beforeEach(() => {
  mockState.reset();
});

// The route goes through the real getSessionFromRequest, so cookie-auth
// DELETEs must carry an allowlisted Origin header (CSRF gate from #615).
function createRequest(options: { origin?: string | null } = {}) {
  const headers = new Headers();
  if (options.origin !== null) {
    headers.set("Origin", options.origin ?? "http://localhost:3000");
  }
  return new Request("http://localhost/api/settings/account", {
    method: "DELETE",
    headers,
  });
}

describe("DELETE /api/settings/account", () => {
  it("returns 401 when session is missing", async () => {
    mockState.getSession.mockResolvedValue(null);

    const response = await DELETE(createRequest());

    expect(response.status).toBe(401);
    expect(await response.json()).toEqual({ error: "Not authenticated" });
    expect(mockState.db.delete).not.toHaveBeenCalled();
    expect(mockState.revalidateUserGroupLeaderboards).not.toHaveBeenCalled();
  });

  it("returns 401 when Origin is missing and never touches the DB", async () => {
    mockState.getSession.mockResolvedValue({
      id: "user-1",
      username: "alice",
      displayName: "Alice",
      avatarUrl: null,
      isAdmin: false,
    });

    const response = await DELETE(createRequest({ origin: null }));

    expect(response.status).toBe(401);
    expect(await response.json()).toEqual({ error: "Not authenticated" });
    expect(mockState.getSession).not.toHaveBeenCalled();
    expect(mockState.db.delete).not.toHaveBeenCalled();
  });

  it("returns 401 when Origin is not allowed and never touches the DB", async () => {
    mockState.getSession.mockResolvedValue({
      id: "user-1",
      username: "alice",
      displayName: "Alice",
      avatarUrl: null,
      isAdmin: false,
    });

    const response = await DELETE(
      createRequest({ origin: "https://attacker.example" })
    );

    expect(response.status).toBe(401);
    expect(await response.json()).toEqual({ error: "Not authenticated" });
    expect(mockState.getSession).not.toHaveBeenCalled();
    expect(mockState.db.delete).not.toHaveBeenCalled();
  });

  it("deletes user account, revalidates caches, and clears session", async () => {
    mockState.getSession.mockResolvedValue({
      id: "user-1",
      username: "Alice",
      displayName: "Alice",
      avatarUrl: null,
      isAdmin: false,
    });
    mockState.setDeletedRows([{ id: "user-1" }]);

    const response = await DELETE(createRequest());

    expect(response.status).toBe(200);
    expect(await response.json()).toEqual({
      success: true,
      deleted: true,
    });
    expect(mockState.db.delete).toHaveBeenCalledTimes(1);
    expect(mockState.eq).toHaveBeenCalledWith("users.id", "user-1");
    expect(mockState.clearSession).toHaveBeenCalledTimes(1);

    // Group leaderboard revalidation must run BEFORE the user row delete —
    // the helper reads group membership rows that the delete cascades away.
    expect(mockState.revalidateUserGroupLeaderboards).toHaveBeenCalledWith(
      "user-1"
    );
    expect(
      mockState.revalidateUserGroupLeaderboards.mock.invocationCallOrder[0]
    ).toBeLessThan(mockState.db.delete.mock.invocationCallOrder[0]);

    expect(mockState.revalidateTag).toHaveBeenCalledTimes(7);
    expect(mockState.revalidateTag).toHaveBeenNthCalledWith(1, "leaderboard", "max");
    expect(mockState.revalidateTag).toHaveBeenNthCalledWith(2, "user:alice", "max");
    expect(mockState.revalidateTag).toHaveBeenNthCalledWith(3, "user-rank", "max");
    expect(mockState.revalidateTag).toHaveBeenNthCalledWith(4, "user-rank:alice", "max");
    expect(mockState.revalidateTag).toHaveBeenNthCalledWith(5, "embed-user:alice", "max");
    expect(mockState.revalidateTag).toHaveBeenNthCalledWith(6, "embed-user:alice:tokens", "max");
    expect(mockState.revalidateTag).toHaveBeenNthCalledWith(7, "embed-user:alice:cost", "max");
    expect(mockState.revalidateUsernamePaths).toHaveBeenCalledWith("Alice");
    expect(mockState.revalidatePath).toHaveBeenNthCalledWith(1, "/leaderboard");
    expect(mockState.revalidatePath).toHaveBeenNthCalledWith(2, "/profile");
  });

  it("returns 404 when user row does not exist", async () => {
    mockState.getSession.mockResolvedValue({
      id: "user-1",
      username: "alice",
      displayName: "Alice",
      avatarUrl: null,
      isAdmin: false,
    });
    mockState.setDeletedRows([]);

    const response = await DELETE(createRequest());

    expect(response.status).toBe(404);
    expect(await response.json()).toEqual({ error: "User not found" });
    expect(mockState.clearSession).not.toHaveBeenCalled();
  });

  it("returns 500 when deletion fails", async () => {
    mockState.getSession.mockResolvedValue({
      id: "user-1",
      username: "alice",
      displayName: "Alice",
      avatarUrl: null,
      isAdmin: false,
    });
    mockState.setDeleteError(new Error("db unavailable"));

    const response = await DELETE(createRequest());

    expect(response.status).toBe(500);
    expect(await response.json()).toEqual({
      error: "Failed to delete account",
    });
  });

  it("still succeeds when clearSession or group revalidation throws", async () => {
    mockState.getSession.mockResolvedValue({
      id: "user-1",
      username: "alice",
      displayName: "Alice",
      avatarUrl: null,
      isAdmin: false,
    });
    mockState.setDeletedRows([{ id: "user-1" }]);
    mockState.clearSession.mockRejectedValue(new Error("cookie error"));
    mockState.revalidateUserGroupLeaderboards.mockRejectedValue(
      new Error("group cache unavailable")
    );

    const response = await DELETE(createRequest());

    expect(response.status).toBe(200);
    expect(await response.json()).toEqual({
      success: true,
      deleted: true,
    });
    expect(mockState.db.delete).toHaveBeenCalledTimes(1);
  });
});
