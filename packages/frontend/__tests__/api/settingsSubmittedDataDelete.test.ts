import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

const mockState = vi.hoisted(() => {
  const getSession = vi.fn();
  const authenticatePersonalToken = vi.fn();
  const revalidateTag = vi.fn();
  const revalidatePath = vi.fn();
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
    authenticatePersonalToken,
    revalidateTag,
    revalidatePath,
    revalidateUsernamePaths,
    eq,
    db,
    where,
    reset() {
      getSession.mockReset();
      authenticatePersonalToken.mockReset();
      revalidateTag.mockReset();
      revalidatePath.mockReset();
      revalidateUsernamePaths.mockReset();
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
}));

vi.mock("@/lib/auth/personalTokens", () => ({
  authenticatePersonalToken: mockState.authenticatePersonalToken,
}));

vi.mock("@/lib/db", () => ({
  db: mockState.db,
  submissions: {
    id: "submissions.id",
    userId: "submissions.userId",
  },
}));

vi.mock("@/lib/db/usernameLookup", () => ({
  normalizeUsernameCacheKey: (username: string) => username.toLowerCase(),
  revalidateUsernamePaths: mockState.revalidateUsernamePaths,
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

function createRequest(token?: string) {
  const headers = new Headers();
  if (token) {
    headers.set("Authorization", `Bearer ${token}`);
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
  });

  it("deletes submitted data and revalidates public caches", async () => {
    mockState.getSession.mockResolvedValue({
      id: "user-1",
      username: "Alice",
      displayName: "Alice",
      avatarUrl: null,
      isAdmin: false,
    });
    mockState.setDeletedRows([{ id: "submission-1" }]);

    const response = await DELETE(createRequest());

    expect(response.status).toBe(200);
    expect(await response.json()).toEqual({
      success: true,
      deleted: true,
      deletedSubmissions: 1,
    });
    expect(mockState.db.delete).toHaveBeenCalledTimes(1);
    expect(mockState.eq).toHaveBeenCalledWith("submissions.userId", "user-1");
    expect(mockState.where).toHaveBeenCalledWith({
      kind: "eq",
      left: "submissions.userId",
      right: "user-1",
    });
    expect(mockState.revalidateTag).toHaveBeenCalledTimes(7);
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
      isAdmin: false,
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
      isAdmin: false,
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

    const response = await DELETE(createRequest("tt_valid"));

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
