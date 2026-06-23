import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

const mockState = vi.hoisted(() => {
  const awaitedResults: unknown[] = [];
  const executeResults: Array<Array<Record<string, unknown>>> = [];
  const limitCalls: unknown[] = [];

  const tables = {
    users: {
      id: "users.id",
      username: "users.username",
      displayName: "users.displayName",
      avatarUrl: "users.avatarUrl",
    },
    submissions: {
      id: "submissions.id",
      userId: "submissions.userId",
      totalTokens: "submissions.totalTokens",
      totalCost: "submissions.totalCost",
      submitCount: "submissions.submitCount",
      updatedAt: "submissions.updatedAt",
    },
    dailyBreakdown: {
      submissionId: "dailyBreakdown.submissionId",
      date: "dailyBreakdown.date",
      tokens: "dailyBreakdown.tokens",
      cost: "dailyBreakdown.cost",
    },
  };

  const eq = vi.fn(() => "eq");
  const and = vi.fn(() => "and");
  const gte = vi.fn(() => "gte");
  const sql = Object.assign(
    vi.fn((strings: TemplateStringsArray, ...values: unknown[]) => ({
      strings: Array.from(strings),
      values,
      as: () => ({}),
    })),
    {
      raw: vi.fn(),
    }
  );

  const db = {
    select: vi.fn(() => {
      const builder = {
        from: vi.fn(() => builder),
        leftJoin: vi.fn(() => builder),
        innerJoin: vi.fn(() => builder),
        where: vi.fn(() => builder),
        groupBy: vi.fn(() => builder),
        orderBy: vi.fn(() => builder),
        limit: vi.fn((value: unknown) => {
          limitCalls.push(value);
          return builder;
        }),
        then: (resolve: (value: unknown) => unknown) =>
          resolve(awaitedResults.shift() ?? []),
      };

      return builder;
    }),
    execute: vi.fn(async () => executeResults.shift() ?? []),
  };

  return {
    db,
    tables,
    eq,
    and,
    gte,
    sql,
    reset() {
      awaitedResults.length = 0;
      executeResults.length = 0;
      limitCalls.length = 0;
      db.select.mockClear();
      db.execute.mockClear();
      eq.mockClear();
      and.mockClear();
      gte.mockClear();
      sql.mockClear();
      sql.raw.mockClear();
    },
    pushAwaitedResult(value: unknown) {
      awaitedResults.push(value);
    },
    pushExecuteResult(rows: Array<Record<string, unknown>>) {
      executeResults.push(rows);
    },
    limitCalls,
  };
});

vi.mock("next/cache", () => ({
  unstable_cache: (fn: () => unknown) => fn,
}));

vi.mock("@/lib/db", () => ({
  db: mockState.db,
  users: mockState.tables.users,
  submissions: mockState.tables.submissions,
  dailyBreakdown: mockState.tables.dailyBreakdown,
}));

vi.mock("@/lib/db/usernameLookup", () => {
  class AmbiguousUsernameError extends Error {}

  return {
    AmbiguousUsernameError,
    USERNAME_LOOKUP_LIMIT: 2,
    getSingleUsernameMatch: (rows: readonly unknown[], username: string) => {
      if (rows.length > 1) {
        throw new AmbiguousUsernameError(`Multiple users match username ${username} case-insensitively`);
      }
      return rows[0] ?? null;
    },
    normalizeUsernameCacheKey: (username: string) => username.toLowerCase(),
    usernameEqualsIgnoreCase: (username: string) =>
      mockState.sql`lower(${mockState.tables.users.username}) = ${username.toLowerCase()}`,
  };
});

vi.mock("drizzle-orm", () => ({
  eq: mockState.eq,
  and: mockState.and,
  gte: mockState.gte,
  sql: mockState.sql,
}));

type ModuleExports = typeof import("../../src/lib/embed/getUserEmbedStats");

let getUserEmbedStats: ModuleExports["getUserEmbedStats"];
let getUserEmbedContributions: ModuleExports["getUserEmbedContributions"];

function serializeSqlCalls(): string[] {
  return mockState.sql.mock.calls.map((call) => {
    const [strings, ...values] = call as [TemplateStringsArray, ...unknown[]];
    const textParts = Array.from(strings);

    return textParts.reduce((text, part, index) => {
      const nextValue = index < values.length ? String(values[index]) : "";
      return `${text}${part}${nextValue}`;
    }, "");
  });
}

beforeAll(async () => {
  const embedModule = await import("../../src/lib/embed/getUserEmbedStats");
  getUserEmbedStats = embedModule.getUserEmbedStats;
  getUserEmbedContributions = embedModule.getUserEmbedContributions;
});

beforeEach(() => {
  mockState.reset();
});

describe("user embed data", () => {
  it("keeps embed tie-breakers out of the rank window", async () => {
    mockState.pushAwaitedResult([
      {
        id: "user-alice",
        username: "alice",
        displayName: "Alice",
        avatarUrl: null,
        totalTokens: 3000,
        totalCost: 40,
        submissionCount: 1,
        updatedAt: new Date("2026-03-12T09:00:00.000Z"),
      },
    ]);
    mockState.pushExecuteResult([{ rank: 2, total: 3 }]);

    await getUserEmbedStats("alice", "tokens");
    const tokenSqlTexts = serializeSqlCalls();

    expect(tokenSqlTexts.some((text) => text.includes("RANK() OVER"))).toBe(true);
    expect(tokenSqlTexts.some((text) =>
      text.includes("total_tokens DESC, CAST(total_cost AS DECIMAL(12,4)) DESC")
    )).toBe(false);

    mockState.reset();
    mockState.pushAwaitedResult([
      {
        id: "user-alice",
        username: "alice",
        displayName: "Alice",
        avatarUrl: null,
        totalTokens: 3000,
        totalCost: 40,
        submissionCount: 1,
        updatedAt: new Date("2026-03-12T09:00:00.000Z"),
      },
    ]);
    mockState.pushExecuteResult([{ rank: 2, total: 3 }]);

    await getUserEmbedStats("alice", "cost");
    const costSqlTexts = serializeSqlCalls();

    expect(costSqlTexts.some((text) => text.includes("RANK() OVER"))).toBe(true);
    expect(costSqlTexts.some((text) =>
      text.includes("CAST(total_cost AS DECIMAL(12,4)) DESC, total_tokens DESC")
    )).toBe(false);
  });

  it("looks up embed stats usernames case-insensitively and returns the canonical username", async () => {
    mockState.pushAwaitedResult([
      {
        id: "user-imlunahey",
        username: "ImLunaHey",
        displayName: "Luna",
        avatarUrl: null,
        totalTokens: 1200,
        totalCost: 12,
        submissionCount: 1,
        updatedAt: new Date("2026-03-12T09:00:00.000Z"),
      },
    ]);
    mockState.pushExecuteResult([{ rank: 4 }]);

    const stats = await getUserEmbedStats("imlunahey", "tokens");
    const sqlTexts = serializeSqlCalls();

    expect(stats?.user.username).toBe("ImLunaHey");
    expect(stats?.stats.rank).toBe(4);
    expect(mockState.limitCalls[0]).toBe(2);
    expect(sqlTexts.some((text) =>
      text.toLowerCase().includes("lower(users.username) = imlunahey")
    )).toBe(true);
  });

  it("looks up embed contributions usernames case-insensitively", async () => {
    mockState.pushAwaitedResult([{ id: "user-imlunahey" }]);
    mockState.pushAwaitedResult([]);

    const contributions = await getUserEmbedContributions("IMLUNAHEY");
    const sqlTexts = serializeSqlCalls();

    expect(contributions).toEqual([]);
    expect(mockState.limitCalls[0]).toBe(2);
    expect(sqlTexts.some((text) =>
      text.toLowerCase().includes("lower(users.username) = imlunahey")
    )).toBe(true);
  });

  it("rejects ambiguous case-insensitive embed stats matches", async () => {
    mockState.pushAwaitedResult([
      {
        id: "user-imlunahey",
        username: "ImLunaHey",
        displayName: "Luna",
        avatarUrl: null,
        totalTokens: 1200,
        totalCost: 12,
        submissionCount: 1,
        updatedAt: new Date("2026-03-12T09:00:00.000Z"),
      },
      {
        id: "user-imlunahey-duplicate",
        username: "imlunahey",
        displayName: "Luna Duplicate",
        avatarUrl: null,
        totalTokens: 100,
        totalCost: 1,
        submissionCount: 1,
        updatedAt: new Date("2026-03-12T09:00:00.000Z"),
      },
    ]);

    await expect(getUserEmbedStats("imlunahey", "tokens")).rejects.toThrow(
      "Multiple users match username imlunahey case-insensitively"
    );
    expect(mockState.limitCalls[0]).toBe(2);
  });
});
