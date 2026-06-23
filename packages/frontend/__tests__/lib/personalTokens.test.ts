import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

const mockState = vi.hoisted(() => {
  const selectResults: Array<Array<Record<string, unknown>>> = [];
  const insertResults: Array<Array<Record<string, unknown>>> = [];
  const updateResults: Array<Array<Record<string, unknown>>> = [];
  const deleteResults: Array<Array<Record<string, unknown>>> = [];
  const insertValues: Array<Record<string, unknown>> = [];
  const updateValues: Array<Record<string, unknown>> = [];
  const executeCalls: unknown[] = [];

  const tables = {
    apiTokens: {
      id: "apiTokens.id",
      userId: "apiTokens.userId",
      token: "apiTokens.token",
      name: "apiTokens.name",
      createdAt: "apiTokens.createdAt",
      lastUsedAt: "apiTokens.lastUsedAt",
      expiresAt: "apiTokens.expiresAt",
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
  const or = vi.fn(() => "or");
  const desc = vi.fn(() => "desc");
  const sql = vi.fn((strings: TemplateStringsArray, ...values: unknown[]) => ({
    strings,
    values,
  }));

  function nextResult<T>(queue: T[][]): T[] {
    return queue.shift() ?? [];
  }

  const execute = vi.fn(async (statement: unknown) => {
    executeCalls.push(statement);
    return [];
  });

  const db = {
    select: vi.fn(() => {
      const builder = {
        from: vi.fn(() => builder),
        innerJoin: vi.fn(() => builder),
        where: vi.fn(() => builder),
        limit: vi.fn(async () => nextResult(selectResults)),
        orderBy: vi.fn(async () => nextResult(selectResults)),
      };

      return builder;
    }),
    insert: vi.fn(() => {
      const builder = {
        values: vi.fn((value: Record<string, unknown>) => {
          insertValues.push(value);
          return builder;
        }),
        returning: vi.fn(async () => nextResult(insertResults)),
      };

      return builder;
    }),
    update: vi.fn(() => {
      const builder = {
        set: vi.fn((value: Record<string, unknown>) => {
          updateValues.push(value);
          return builder;
        }),
        where: vi.fn(async () => nextResult(updateResults)),
      };

      return builder;
    }),
    delete: vi.fn(() => {
      const builder = {
        where: vi.fn(() => builder),
        returning: vi.fn(async () => nextResult(deleteResults)),
      };

      return builder;
    }),
    execute,
    transaction: vi.fn(async (callback: (tx: unknown) => Promise<unknown>) => callback(tx)),
  };

  const tx = {
    select: db.select,
    insert: db.insert,
    update: db.update,
    delete: db.delete,
    execute,
  };

  return {
    db,
    tables,
    eq,
    and,
    or,
    desc,
    sql,
    insertValues,
    updateValues,
    executeCalls,
    reset() {
      selectResults.length = 0;
      insertResults.length = 0;
      updateResults.length = 0;
      deleteResults.length = 0;
      insertValues.length = 0;
      updateValues.length = 0;
      executeCalls.length = 0;
      db.select.mockClear();
      db.insert.mockClear();
      db.update.mockClear();
      db.delete.mockClear();
      db.execute.mockClear();
      db.transaction.mockClear();
      eq.mockClear();
      and.mockClear();
      or.mockClear();
      desc.mockClear();
      sql.mockClear();
    },
    pushSelectResult(rows: Array<Record<string, unknown>>) {
      selectResults.push(rows);
    },
    pushInsertResult(rows: Array<Record<string, unknown>>) {
      insertResults.push(rows);
    },
    pushUpdateResult(rows: Array<Record<string, unknown>> = []) {
      updateResults.push(rows);
    },
    pushDeleteResult(rows: Array<Record<string, unknown>>) {
      deleteResults.push(rows);
    },
  };
});

const generateApiToken = vi.fn(() => "tt_test_token");
const hashToken = vi.fn((token: string) => `hashed_${token}`);

vi.mock("@/lib/db", () => ({
  db: mockState.db,
  apiTokens: mockState.tables.apiTokens,
  users: mockState.tables.users,
}));

vi.mock("drizzle-orm", () => ({
  eq: mockState.eq,
  and: mockState.and,
  or: mockState.or,
  desc: mockState.desc,
  sql: mockState.sql,
}));

vi.mock("@/lib/auth/utils", () => ({
  generateApiToken,
  hashToken,
}));

type ModuleExports = typeof import("../../src/lib/auth/personalTokens");

let issuePersonalToken: ModuleExports["issuePersonalToken"];
let authenticatePersonalToken: ModuleExports["authenticatePersonalToken"];
let listPersonalTokens: ModuleExports["listPersonalTokens"];
let revokePersonalToken: ModuleExports["revokePersonalToken"];

beforeAll(async () => {
  const personalTokensModule = await import("../../src/lib/auth/personalTokens");
  issuePersonalToken = personalTokensModule.issuePersonalToken;
  authenticatePersonalToken = personalTokensModule.authenticatePersonalToken;
  listPersonalTokens = personalTokensModule.listPersonalTokens;
  revokePersonalToken = personalTokensModule.revokePersonalToken;
});

beforeEach(() => {
  mockState.reset();
  generateApiToken.mockClear();
  generateApiToken.mockReturnValue("tt_test_token");
  hashToken.mockClear();
  hashToken.mockImplementation((token: string) => `hashed_${token}`);
  vi.useRealTimers();
});

describe("personal token service", () => {
  it("issues a token and keeps the requested name when it is available", async () => {
    mockState.pushSelectResult([]);
    mockState.pushInsertResult([
      {
        id: "token-1",
        userId: "user-1",
        name: "CLI",
        createdAt: new Date("2026-03-08T04:00:00.000Z"),
        lastUsedAt: null,
        expiresAt: null,
      },
    ]);

    const token = await issuePersonalToken({
      userId: "user-1",
      name: "CLI",
      ensureUniqueName: true,
    });

    expect(token).toMatchObject({
      id: "token-1",
      userId: "user-1",
      name: "CLI",
      token: "tt_test_token",
    });
    expect(mockState.insertValues[0]).toMatchObject({
      userId: "user-1",
      name: "CLI",
      token: "hashed_tt_test_token",
      expiresAt: null,
    });
    expect(mockState.db.transaction).toHaveBeenCalledTimes(1);
    expect(mockState.executeCalls).toHaveLength(1);
  });

  it("suffixes duplicate token names when requested", async () => {
    mockState.pushSelectResult([
      { name: "CLI" },
      { name: "CLI (1)" },
    ]);
    mockState.pushInsertResult([
      {
        id: "token-2",
        userId: "user-1",
        name: "CLI (2)",
        createdAt: new Date("2026-03-08T04:00:00.000Z"),
        lastUsedAt: null,
        expiresAt: null,
      },
    ]);

    const token = await issuePersonalToken({
      userId: "user-1",
      name: "CLI",
      ensureUniqueName: true,
    });

    expect(token.name).toBe("CLI (2)");
    expect(mockState.insertValues[0]).toMatchObject({
      name: "CLI (2)",
    });
    expect(mockState.db.transaction).toHaveBeenCalledTimes(1);
    expect(mockState.executeCalls).toHaveLength(1);
  });

  it("keeps incrementing the numeric suffix under the advisory lock", async () => {
    mockState.pushSelectResult([
      { name: "CLI" },
      { name: "CLI (1)" },
      { name: "CLI (2)" },
    ]);
    mockState.pushInsertResult([
      {
        id: "token-3",
        userId: "user-1",
        name: "CLI (3)",
        createdAt: new Date("2026-03-08T04:00:00.000Z"),
        lastUsedAt: null,
        expiresAt: null,
      },
    ]);

    const token = await issuePersonalToken({
      userId: "user-1",
      name: "CLI",
      ensureUniqueName: true,
    });

    expect(token.name).toBe("CLI (3)");
    expect(mockState.db.insert).toHaveBeenCalledTimes(1);
    expect(mockState.db.transaction).toHaveBeenCalledTimes(1);
    expect(mockState.executeCalls).toHaveLength(1);
    expect(mockState.insertValues[0]).toMatchObject({
      name: "CLI (3)",
    });
  });

  it("can create a token without taking the advisory lock", async () => {
    mockState.pushInsertResult([
      {
        id: "token-raw",
        userId: "user-1",
        name: "CLI raw",
        createdAt: new Date("2026-03-08T04:00:00.000Z"),
        lastUsedAt: null,
        expiresAt: null,
      },
    ]);

    const token = await issuePersonalToken({
      userId: "user-1",
      name: "CLI raw",
    });

    expect(token.name).toBe("CLI raw");
    expect(mockState.db.transaction).not.toHaveBeenCalled();
    expect(mockState.executeCalls).toHaveLength(0);
    expect(mockState.insertValues[0]).toMatchObject({
      name: "CLI raw",
    });
  });

  it("returns invalid for malformed tokens without hitting the database", async () => {
    const result = await authenticatePersonalToken("not-a-token");

    expect(result).toEqual({ status: "invalid" });
    expect(mockState.db.select).not.toHaveBeenCalled();
  });

  it("returns expired when the token exists but is past its expiry", async () => {
    mockState.pushSelectResult([
      {
        tokenId: "token-1",
        tokenValue: "hashed_tt_test_token",
        userId: "user-1",
        username: "alice",
        displayName: "Alice",
        avatarUrl: null,
        expiresAt: new Date("2026-03-01T00:00:00.000Z"),
      },
    ]);

    const result = await authenticatePersonalToken("tt_test_token");

    expect(result).toEqual({ status: "expired" });
    expect(mockState.db.update).not.toHaveBeenCalled();
  });

  it("treats a token expiring right now as expired", async () => {
    const now = new Date("2026-03-08T04:00:00.000Z");
    vi.useFakeTimers();
    vi.setSystemTime(now);

    mockState.pushSelectResult([
      {
        tokenId: "token-1",
        tokenValue: "hashed_tt_test_token",
        userId: "user-1",
        username: "alice",
        displayName: "Alice",
        avatarUrl: null,
        expiresAt: now,
      },
    ]);

    const result = await authenticatePersonalToken("tt_test_token");

    expect(result).toEqual({ status: "expired" });
    expect(mockState.db.update).not.toHaveBeenCalled();
  });

  it("returns the user and touches lastUsedAt for a valid token", async () => {
    mockState.pushSelectResult([
      {
        tokenId: "token-1",
        tokenValue: "hashed_tt_test_token",
        userId: "user-1",
        username: "alice",
        displayName: "Alice",
        avatarUrl: null,
        expiresAt: null,
      },
    ]);
    mockState.pushUpdateResult();

    const result = await authenticatePersonalToken("tt_test_token");

    expect(result).toMatchObject({
      status: "valid",
      tokenId: "token-1",
      userId: "user-1",
      username: "alice",
    });
    expect(mockState.db.update).toHaveBeenCalledTimes(1);
    expect(mockState.updateValues[0]).toHaveProperty("lastUsedAt");
  });

  it("can skip touching lastUsedAt when the caller opts out", async () => {
    mockState.pushSelectResult([
      {
        tokenId: "token-1",
        tokenValue: "hashed_tt_test_token",
        userId: "user-1",
        username: "alice",
        displayName: "Alice",
        avatarUrl: null,
        expiresAt: null,
      },
    ]);

    const result = await authenticatePersonalToken("tt_test_token", {
      touchLastUsedAt: false,
    });

    expect(result).toMatchObject({
      status: "valid",
      tokenId: "token-1",
    });
    expect(mockState.db.update).not.toHaveBeenCalled();
  });

  it("lists tokens for a user", async () => {
    mockState.pushSelectResult([
      {
        id: "token-2",
        userId: "user-1",
        name: "CLI",
        createdAt: new Date("2026-03-08T04:00:00.000Z"),
        lastUsedAt: null,
        expiresAt: null,
      },
    ]);

    const tokens = await listPersonalTokens("user-1");

    expect(tokens).toHaveLength(1);
    expect(tokens[0]).toMatchObject({
      id: "token-2",
      name: "CLI",
    });
  });

  it("revokes only matching user tokens", async () => {
    mockState.pushDeleteResult([{ id: "token-1" }]);

    const revoked = await revokePersonalToken("user-1", "token-1");

    expect(revoked).toBe(true);
    expect(mockState.db.delete).toHaveBeenCalledTimes(1);
    expect(mockState.eq).toHaveBeenNthCalledWith(1, mockState.tables.apiTokens.id, "token-1");
    expect(mockState.eq).toHaveBeenNthCalledWith(2, mockState.tables.apiTokens.userId, "user-1");
    expect(mockState.and).toHaveBeenCalledTimes(1);
  });
});
