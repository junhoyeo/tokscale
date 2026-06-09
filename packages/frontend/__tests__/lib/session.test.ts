import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

const mockState = vi.hoisted(() => {
  const selectResults: Array<Array<Record<string, unknown>>> = [];
  const insertValues: Array<Record<string, unknown>> = [];
  const cookieStore = {
    get: vi.fn(),
    set: vi.fn(),
    delete: vi.fn(),
  };

  const tables = {
    sessions: {
      id: "sessions.id",
      userId: "sessions.userId",
      tokenHash: "sessions.tokenHash",
      expiresAt: "sessions.expiresAt",
      source: "sessions.source",
      userAgent: "sessions.userAgent",
    },
    users: {
      id: "users.id",
      username: "users.username",
      displayName: "users.displayName",
      avatarUrl: "users.avatarUrl",
    },
  };

  const eq = vi.fn(() => "eq");
  const and = vi.fn(() => "and");
  const gt = vi.fn(() => "gt");

  function nextResult<T>(queue: T[][]): T[] {
    return queue.shift() ?? [];
  }

  const db = {
    select: vi.fn(() => {
      const builder = {
        from: vi.fn(() => builder),
        innerJoin: vi.fn(() => builder),
        where: vi.fn(() => builder),
        limit: vi.fn(async () => nextResult(selectResults)),
      };

      return builder;
    }),
    insert: vi.fn(() => {
      const builder = {
        values: vi.fn((value: Record<string, unknown>) => {
          insertValues.push(value);
          return builder;
        }),
      };

      return builder;
    }),
    delete: vi.fn(() => {
      const builder = {
        where: vi.fn(() => builder),
      };

      return builder;
    }),
  };

  return {
    db,
    tables,
    eq,
    and,
    gt,
    cookieStore,
    insertValues,
    reset() {
      selectResults.length = 0;
      insertValues.length = 0;
      db.select.mockClear();
      db.insert.mockClear();
      db.delete.mockClear();
      eq.mockClear();
      and.mockClear();
      gt.mockClear();
      cookieStore.get.mockReset();
      cookieStore.set.mockReset();
      cookieStore.delete.mockReset();
    },
    pushSelectResult(rows: Array<Record<string, unknown>>) {
      selectResults.push(rows);
    },
  };
});

const generateRandomString = vi.fn(() => "plain_session_token");
const hashToken = vi.fn((token: string) => `hashed_${token}`);
const authenticatePersonalToken = vi.fn();

vi.mock("next/headers", () => ({
  cookies: vi.fn(async () => mockState.cookieStore),
}));

vi.mock("@/lib/db", () => ({
  db: mockState.db,
  sessions: mockState.tables.sessions,
  users: mockState.tables.users,
}));

vi.mock("drizzle-orm", () => ({
  eq: mockState.eq,
  and: mockState.and,
  gt: mockState.gt,
}));

vi.mock("@/lib/auth/utils", () => ({
  generateRandomString,
  hashToken,
}));

vi.mock("@/lib/auth/personalTokens", () => ({
  authenticatePersonalToken,
}));

type ModuleExports = typeof import("../../src/lib/auth/session");

let createSession: ModuleExports["createSession"];
let getSession: ModuleExports["getSession"];
let clearSession: ModuleExports["clearSession"];
let getSessionFromHeader: ModuleExports["getSessionFromHeader"];

beforeAll(async () => {
  const sessionModule = await import("../../src/lib/auth/session");
  createSession = sessionModule.createSession;
  getSession = sessionModule.getSession;
  clearSession = sessionModule.clearSession;
  getSessionFromHeader = sessionModule.getSessionFromHeader;
});

beforeEach(() => {
  mockState.reset();
  generateRandomString.mockClear();
  generateRandomString.mockReturnValue("plain_session_token");
  hashToken.mockClear();
  hashToken.mockImplementation((token: string) => `hashed_${token}`);
  authenticatePersonalToken.mockReset();
});

describe("browser sessions", () => {
  it("stores a hash while returning the plaintext session token", async () => {
    const token = await createSession("user-1", {
      source: "web",
      userAgent: "Vitest",
    });

    expect(token).toBe("plain_session_token");
    expect(mockState.insertValues[0]).toMatchObject({
      userId: "user-1",
      tokenHash: "hashed_plain_session_token",
      source: "web",
      userAgent: "Vitest",
    });
    expect(mockState.insertValues[0]).not.toHaveProperty("token", "plain_session_token");
    expect(hashToken).toHaveBeenCalledWith("plain_session_token");
  });

  it("looks up cookie sessions by hashing the plaintext cookie value", async () => {
    mockState.cookieStore.get.mockReturnValue({ value: "plain_cookie_token" });
    mockState.pushSelectResult([
      {
        user: {
          id: "user-1",
          username: "alice",
          displayName: "Alice",
          avatarUrl: null,
        },
      },
    ]);

    const session = await getSession();

    expect(session).toEqual({
      id: "user-1",
      username: "alice",
      displayName: "Alice",
      avatarUrl: null,
    });
    expect(hashToken).toHaveBeenCalledWith("plain_cookie_token");
    expect(mockState.eq).toHaveBeenCalledWith(
      mockState.tables.sessions.tokenHash,
      "hashed_plain_cookie_token"
    );
  });

  it("deletes sessions by hashing the plaintext cookie value", async () => {
    mockState.cookieStore.get.mockReturnValue({ value: "plain_cookie_token" });

    await clearSession();

    expect(hashToken).toHaveBeenCalledWith("plain_cookie_token");
    expect(mockState.eq).toHaveBeenCalledWith(
      mockState.tables.sessions.tokenHash,
      "hashed_plain_cookie_token"
    );
    expect(mockState.cookieStore.delete).toHaveBeenCalledWith("tt_session");
  });

  it("rejects raw web session tokens sent through Authorization", async () => {
    const session = await getSessionFromHeader(
      new Request("http://localhost.test/api", {
        headers: { Authorization: "Bearer plain_session_token" },
      })
    );

    expect(session).toBeNull();
    expect(mockState.db.select).not.toHaveBeenCalled();
    expect(authenticatePersonalToken).not.toHaveBeenCalled();
  });

  it("still authenticates personal tokens from Authorization", async () => {
    authenticatePersonalToken.mockResolvedValue({
      status: "valid",
      tokenId: "token-1",
      userId: "user-1",
      username: "alice",
      displayName: "Alice",
      avatarUrl: null,
      expiresAt: null,
    });

    const session = await getSessionFromHeader(
      new Request("http://localhost.test/api", {
        headers: { Authorization: "Bearer tt_personal_token" },
      })
    );

    expect(session).toEqual({
      id: "user-1",
      username: "alice",
      displayName: "Alice",
      avatarUrl: null,
    });
    expect(authenticatePersonalToken).toHaveBeenCalledWith("tt_personal_token");
  });
});
