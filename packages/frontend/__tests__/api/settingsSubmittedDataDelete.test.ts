import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

const mockState = vi.hoisted(() => {
  const getSession = vi.fn();
  const getSessionFromHeader = vi.fn();
  const authenticatePersonalToken = vi.fn();
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

  const deleteFromTable = vi.fn(() => ({
    where,
  }));
  const db = {
    delete: deleteFromTable,
    transaction: vi.fn(async (callback: (tx: { delete: typeof deleteFromTable }) => Promise<unknown>) =>
      callback(db)
    ),
  };

  const tables = {
    submissions: {
      id: "submissions.id",
      userId: "submissions.userId",
    },
    submittedDevices: {
      userId: "submittedDevices.userId",
    },
  };

  return {
    getSession,
    getSessionFromHeader,
    authenticatePersonalToken,
    revalidateTag,
    revalidatePath,
    revalidateUserGroupLeaderboards,
    revalidateUsernamePaths,
    eq,
    db,
    tables,
    where,
    reset() {
      getSession.mockReset();
      getSessionFromHeader.mockReset();
      authenticatePersonalToken.mockReset();
      revalidateTag.mockReset();
      revalidatePath.mockReset();
      revalidateUserGroupLeaderboards.mockReset();
      revalidateUsernamePaths.mockReset();
      eq.mockClear();
      db.delete.mockClear();
      db.transaction.mockClear();
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
}));

vi.mock("@/lib/auth/personalTokens", () => ({
  authenticatePersonalToken: mockState.authenticatePersonalToken,
}));

vi.mock("@/lib/db", () => ({
  db: mockState.db,
  submissions: mockState.tables.submissions,
  submittedDevices: mockState.tables.submittedDevices,
}));

vi.mock("@/lib/db/usernameLookup", () => ({
  normalizeUsernameCacheKey: (username: string) => username.toLowerCase(),
  revalidateUsernamePaths: mockState.revalidateUsernamePaths,
}));

vi.mock("@/lib/groups/cache", () => ({
  revalidateUserGroupLeaderboards: mockState.revalidateUserGroupLeaderboards,
}));

type ModuleExports = typeof import("../../src/app/api/settings/submitted-data/route");

let DELETE: ModuleExports["DELETE"];

beforeAll(async () => {
  const routeModule = await import("../../src/app/api/settings/submitted-data/route");
  DELETE = routeModule.DELETE;
});

beforeEach(() => {
  mockState.reset();
});

function createRequest(options: { token?: string; origin?: string | null } = {}) {
  const headers = new Headers();
  if (options.token) {
    headers.set("Authorization", `Bearer ${options.token}`);
  }
  if (options.origin !== null) {
    headers.set("Origin", options.origin ?? "http://localhost:3000");
  }
  return new Request("http://localhost/api/settings/submitted-data", {
    method: "DELETE",
    headers,
  });
}

describe("DELETE /api/settings/submitted-data", () => {
  it("returns 401 when session is missing", async () => {
    mockState.getSession.mockResolvedValue(null);

    const response = await DELETE(createRequest());

    expect(response.status).toBe(401);
    expect(await response.json()).toEqual({ error: "Not authenticated" });
    expect(mockState.db.delete).not.toHaveBeenCalled();
    expect(mockState.db.transaction).not.toHaveBeenCalled();
  });

  it("returns 401 for cookie auth when Origin is missing", async () => {
    mockState.getSession.mockResolvedValue({
      id: "user-1",
      username: "alice",
      displayName: "Alice",
      avatarUrl: null,
    });

    const response = await DELETE(createRequest({ origin: null }));

    expect(response.status).toBe(401);
    expect(await response.json()).toEqual({ error: "Not authenticated" });
    expect(mockState.getSession).not.toHaveBeenCalled();
    expect(mockState.db.delete).not.toHaveBeenCalled();
    expect(mockState.db.transaction).not.toHaveBeenCalled();
  });

  it("returns 401 for cookie auth when Origin is not allowed", async () => {
    mockState.getSession.mockResolvedValue({
      id: "user-1",
      username: "alice",
      displayName: "Alice",
      avatarUrl: null,
    });

    const response = await DELETE(
      createRequest({ origin: "https://attacker.example" })
    );

    expect(response.status).toBe(401);
    expect(await response.json()).toEqual({ error: "Not authenticated" });
    expect(mockState.getSession).not.toHaveBeenCalled();
    expect(mockState.db.delete).not.toHaveBeenCalled();
    expect(mockState.db.transaction).not.toHaveBeenCalled();
  });

  it("deletes submitted data and revalidates public caches", async () => {
    mockState.getSession.mockResolvedValue({
      id: "user-1",
      username: "Alice",
      displayName: "Alice",
      avatarUrl: null,
    });
    mockState.setDeletedRows([{ id: "submission-1" }]);
    mockState.revalidateUserGroupLeaderboards.mockRejectedValueOnce(
      new Error("group cache unavailable")
    );

    const response = await DELETE(createRequest());

    expect(response.status).toBe(200);
    expect(await response.json()).toEqual({
      success: true,
      deleted: true,
      deletedSubmissions: 1,
    });
    expect(mockState.db.delete).toHaveBeenCalledTimes(2);
    expect(mockState.db.transaction).toHaveBeenCalledTimes(1);
    expect(mockState.db.delete).toHaveBeenNthCalledWith(1, mockState.tables.submissions);
    expect(mockState.db.delete).toHaveBeenNthCalledWith(2, mockState.tables.submittedDevices);
    expect(mockState.eq).toHaveBeenNthCalledWith(1, "submissions.userId", "user-1");
    expect(mockState.eq).toHaveBeenNthCalledWith(2, "submittedDevices.userId", "user-1");
    expect(mockState.where).toHaveBeenCalledWith({
      kind: "eq",
      left: "submissions.userId",
      right: "user-1",
    });
    expect(mockState.revalidateTag).toHaveBeenCalledTimes(7);
    expect(mockState.revalidateUserGroupLeaderboards).toHaveBeenCalledWith("user-1");
    expect(mockState.revalidateUsernamePaths).toHaveBeenCalledTimes(1);
    expect(mockState.revalidateUsernamePaths).toHaveBeenCalledWith("Alice");
    expect(mockState.revalidatePath).toHaveBeenCalledTimes(8);
    expect(mockState.revalidateTag).toHaveBeenNthCalledWith(1, "leaderboard", "max");
    expect(mockState.revalidateTag).toHaveBeenNthCalledWith(2, "user:alice", "max");
    expect(mockState.revalidateTag).toHaveBeenNthCalledWith(3, "user-rank", "max");
    expect(mockState.revalidateTag).toHaveBeenNthCalledWith(4, "user-rank:alice", "max");
    expect(mockState.revalidateTag).toHaveBeenNthCalledWith(5, "embed-user:alice", "max");
    expect(mockState.revalidateTag).toHaveBeenNthCalledWith(6, "embed-user:alice:tokens", "max");
    expect(mockState.revalidateTag).toHaveBeenNthCalledWith(7, "embed-user:alice:cost", "max");
    expect(mockState.revalidatePath).toHaveBeenNthCalledWith(1, "/leaderboard");
    expect(mockState.revalidatePath).toHaveBeenNthCalledWith(2, "/profile");
    expect(mockState.revalidatePath).toHaveBeenNthCalledWith(3, "/u/Alice");
    expect(mockState.revalidatePath).toHaveBeenNthCalledWith(4, "/api/users/Alice");
    expect(mockState.revalidatePath).toHaveBeenNthCalledWith(5, "/api/embed/Alice/svg");
    expect(mockState.revalidatePath).toHaveBeenNthCalledWith(6, "/u/alice");
    expect(mockState.revalidatePath).toHaveBeenNthCalledWith(7, "/api/users/alice");
    expect(mockState.revalidatePath).toHaveBeenNthCalledWith(8, "/api/embed/alice/svg");
  });

  it("returns success and still revalidates caches when no submitted data exists", async () => {
    mockState.getSession.mockResolvedValue({
      id: "user-1",
      username: "alice",
      displayName: "Alice",
      avatarUrl: null,
    });
    mockState.setDeletedRows([]);

    const response = await DELETE(createRequest());

    expect(response.status).toBe(200);
    expect(await response.json()).toEqual({
      success: true,
      deleted: false,
      deletedSubmissions: 0,
    });
    expect(mockState.revalidateTag).toHaveBeenCalledWith("leaderboard", "max");
    expect(mockState.revalidateUsernamePaths).toHaveBeenCalledWith("alice");
    expect(mockState.revalidatePath).toHaveBeenCalledTimes(5);
    expect(mockState.revalidatePath).toHaveBeenNthCalledWith(1, "/leaderboard");
    expect(mockState.revalidatePath).toHaveBeenNthCalledWith(2, "/profile");
    expect(mockState.revalidatePath).toHaveBeenNthCalledWith(3, "/u/alice");
    expect(mockState.revalidatePath).toHaveBeenNthCalledWith(4, "/api/users/alice");
    expect(mockState.revalidatePath).toHaveBeenNthCalledWith(5, "/api/embed/alice/svg");
  });

  it("returns 500 when deletion fails", async () => {
    mockState.getSession.mockResolvedValue({
      id: "user-1",
      username: "alice",
      displayName: "Alice",
      avatarUrl: null,
    });
    mockState.setDeleteError(new Error("db unavailable"));

    const response = await DELETE(createRequest());

    expect(response.status).toBe(500);
    expect(await response.json()).toEqual({
      error: "Failed to delete submitted usage data",
    });
  });

  it("accepts bearer token auth for CLI deletion", async () => {
    mockState.authenticatePersonalToken.mockResolvedValue({
      status: "valid",
      userId: "user-2",
      username: "bob",
    });
    mockState.setDeletedRows([{ id: "submission-2" }]);

    const response = await DELETE(
      createRequest({
        token: "tt_valid",
        origin: "https://attacker.example",
      })
    );

    expect(response.status).toBe(200);
    expect(await response.json()).toEqual({
      success: true,
      deleted: true,
      deletedSubmissions: 1,
    });
    expect(mockState.authenticatePersonalToken).toHaveBeenCalledWith("tt_valid", {
      touchLastUsedAt: false,
    });
    expect(mockState.getSession).not.toHaveBeenCalled();
    expect(mockState.where).toHaveBeenCalledWith({
      kind: "eq",
      left: "submissions.userId",
      right: "user-2",
    });
  });
});
