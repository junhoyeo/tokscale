import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

import { expectNoNarrowedCostCast } from "../support/costCastWidths";

const mockState = vi.hoisted(() => {
  const awaitedResults: unknown[] = [];
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
    },
    dailyBreakdown: {
      submissionId: "dailyBreakdown.submissionId",
      date: "dailyBreakdown.date",
      tokens: "dailyBreakdown.tokens",
      cost: "dailyBreakdown.cost",
    },
  };

  const eq = vi.fn(() => "eq");
  const desc = vi.fn(() => "desc");
  const and = vi.fn(() => "and");
  const or = vi.fn(() => "or");
  const gte = vi.fn(() => "gte");
  const lte = vi.fn(() => "lte");
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
        innerJoin: vi.fn(() => builder),
        where: vi.fn(() => builder),
        groupBy: vi.fn(() => builder),
        orderBy: vi.fn(() => builder),
        limit: vi.fn((value: unknown) => {
          limitCalls.push(value);
          return builder;
        }),
        offset: vi.fn(() => builder),
        having: vi.fn(() => builder),
        as: vi.fn(() => builder),
        then: (resolve: (value: unknown) => unknown) =>
          resolve(awaitedResults.shift() ?? []),
      };

      return builder;
    }),
  };

  return {
    db,
    tables,
    eq,
    desc,
    and,
    or,
    gte,
    lte,
    sql,
    reset() {
      awaitedResults.length = 0;
      limitCalls.length = 0;
      db.select.mockClear();
      eq.mockClear();
      desc.mockClear();
      and.mockClear();
      or.mockClear();
      gte.mockClear();
      lte.mockClear();
      sql.mockClear();
      sql.raw.mockClear();
    },
    pushAwaitedResult(value: unknown) {
      awaitedResults.push(value);
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
  desc: mockState.desc,
  and: mockState.and,
  or: mockState.or,
  gte: mockState.gte,
  lte: mockState.lte,
  sql: mockState.sql,
}));

type ModuleExports = typeof import("../../src/lib/leaderboard/getLeaderboard");

let getLeaderboardData: ModuleExports["getLeaderboardData"];
let getUserRank: ModuleExports["getUserRank"];

function selectedKeys(callIndex: number): string[] {
  const calls = mockState.db.select.mock.calls as unknown as Array<
    [Record<string, unknown> | undefined]
  >;
  return Object.keys(calls[callIndex]?.[0] ?? {});
}

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
  const leaderboardModule = await import("../../src/lib/leaderboard/getLeaderboard");
  getLeaderboardData = leaderboardModule.getLeaderboardData;
  getUserRank = leaderboardModule.getUserRank;
});

beforeEach(() => {
  mockState.reset();
});

afterEach(() => {
  vi.useRealTimers();
});

describe("all-time leaderboard queries", () => {
  it("uses competition-rank SQL for all-time list and search ranks", async () => {
    mockState.pushAwaitedResult([]);
    mockState.pushAwaitedResult([{ totalTokens: 0, totalCost: 0, uniqueUsers: 0 }]);

    await getLeaderboardData("all", 1, 50, "tokens");
    const listSqlTexts = serializeSqlCalls();

    expect(listSqlTexts.some((text) => text.includes("RANK() OVER (ORDER BY"))).toBe(true);
    expect(listSqlTexts.some((text) => text.includes("ROW_NUMBER() OVER"))).toBe(false);

    mockState.reset();
    mockState.pushAwaitedResult([]);
    mockState.pushAwaitedResult([{ count: 0 }]);
    mockState.pushAwaitedResult([{ totalTokens: 0, totalCost: 0, uniqueUsers: 0 }]);

    await getLeaderboardData("all", 1, 50, "tokens", "ali");
    const searchSqlTexts = serializeSqlCalls();

    expect(searchSqlTexts.some((text) => text.includes("RANK() OVER (ORDER BY"))).toBe(true);
    expect(searchSqlTexts.some((text) => text.includes("ROW_NUMBER() OVER"))).toBe(false);
  });

  it("counts distinct users for all-time pagination and global stats", async () => {
    mockState.pushAwaitedResult([]);
    mockState.pushAwaitedResult([{ totalTokens: 0, totalCost: 0, uniqueUsers: 2 }]);

    const list = await getLeaderboardData("all", 1, 50, "tokens");
    expect(list.pagination.totalUsers).toBe(2);
    expect(serializeSqlCalls().some((text) =>
      text.includes("COUNT(DISTINCT submissions.userId)")
    )).toBe(true);

    mockState.reset();
    mockState.pushAwaitedResult([]);
    mockState.pushAwaitedResult([{ count: 0 }]);
    mockState.pushAwaitedResult([{ totalTokens: 0, totalCost: 0, uniqueUsers: 2 }]);

    const search = await getLeaderboardData("all", 1, 50, "tokens", "ali");
    expect(search.stats.uniqueUsers).toBe(2);
    expect(serializeSqlCalls().some((text) =>
      text.includes("COUNT(DISTINCT submissions.userId)")
    )).toBe(true);
  });

  it("keeps tied all-time users at the same rank across list, search, and user rank", async () => {
    mockState.pushAwaitedResult([
      {
        rank: 1,
        userId: "user-bob",
        username: "bob",
        displayName: "Bob",
        avatarUrl: null,
        totalTokens: 5000,
        totalCost: 50,
      },
      {
        rank: 2,
        userId: "user-alice",
        username: "alice",
        displayName: "Alice",
        avatarUrl: null,
        totalTokens: 3000,
        totalCost: 40,
      },
      {
        rank: 2,
        userId: "user-alicia",
        username: "alicia",
        displayName: "Alicia",
        avatarUrl: null,
        totalTokens: 3000,
        totalCost: 30,
      },
    ]);
    mockState.pushAwaitedResult([
      {
        totalTokens: 11000,
        totalCost: 120,
        uniqueUsers: 3,
      },
    ]);

    const leaderboard = await getLeaderboardData("all", 1, 50, "tokens");
    const aliceListRank = leaderboard.users.find((user) => user.username === "alice")?.rank;
    const aliciaListRank = leaderboard.users.find((user) => user.username === "alicia")?.rank;

    mockState.reset();
    mockState.pushAwaitedResult([
      {
        rank: 2,
        userId: "user-alice",
        username: "alice",
        displayName: "Alice",
        avatarUrl: null,
        totalTokens: 3000,
        totalCost: 40,
      },
      {
        rank: 2,
        userId: "user-alicia",
        username: "alicia",
        displayName: "Alicia",
        avatarUrl: null,
        totalTokens: 3000,
        totalCost: 30,
      },
    ]);
    mockState.pushAwaitedResult([{ count: 2 }]);
    mockState.pushAwaitedResult([
      {
        totalTokens: 11000,
        totalCost: 120,
        uniqueUsers: 3,
      },
    ]);

    const searchLeaderboard = await getLeaderboardData("all", 1, 50, "tokens", "ali");
    const aliceSearchRank = searchLeaderboard.users.find((user) => user.username === "alice")?.rank;
    const aliciaSearchRank = searchLeaderboard.users.find((user) => user.username === "alicia")?.rank;

    mockState.reset();
    mockState.pushAwaitedResult([
      {
        id: "user-alice",
        username: "alice",
        displayName: "Alice",
        avatarUrl: null,
      },
    ]);
    mockState.pushAwaitedResult([
      {
        totalTokens: 3000,
        totalCost: 40,
      },
    ]);
    mockState.pushAwaitedResult([{ count: 1 }]);

    const aliceUserRank = await getUserRank("alice", "all", "tokens");

    expect(aliceListRank).toBe(2);
    expect(aliciaListRank).toBe(2);
    expect(aliceSearchRank).toBe(2);
    expect(aliciaSearchRank).toBe(2);
    expect(aliceUserRank?.rank).toBe(2);
  });

  it("selects and returns only fields consumed by leaderboard surfaces", async () => {
    mockState.pushAwaitedResult([
      {
        rank: 1,
        userId: "user-alice",
        username: "alice",
        displayName: "Alice",
        avatarUrl: null,
        totalTokens: 3000,
        totalCost: 30,
      },
    ]);
    mockState.pushAwaitedResult([
      {
        totalTokens: 3000,
        totalCost: 30,
        uniqueUsers: 1,
      },
    ]);

    const leaderboard = await getLeaderboardData("all", 1, 50, "tokens");
    expect(selectedKeys(0)).toEqual([
      "rank",
      "userId",
      "username",
      "displayName",
      "avatarUrl",
      "totalTokens",
      "totalCost",
    ]);
    expect(selectedKeys(1)).toEqual([
      "totalTokens",
      "totalCost",
      "uniqueUsers",
    ]);
    expect(Object.keys(leaderboard.users[0]).sort()).toEqual([
      "avatarUrl",
      "displayName",
      "rank",
      "totalCost",
      "totalTokens",
      "userId",
      "username",
    ]);
    expect(leaderboard.stats).toEqual({
      totalTokens: 3000,
      totalCost: 30,
      uniqueUsers: 1,
    });

    mockState.reset();

    mockState.pushAwaitedResult([
      {
        id: "user-alice",
        username: "alice",
        displayName: "Alice",
        avatarUrl: null,
      },
    ]);
    mockState.pushAwaitedResult([
      {
        totalTokens: 3000,
        totalCost: 30,
      },
    ]);
    mockState.pushAwaitedResult([
      {
        count: 0,
      },
    ]);

    const rank = await getUserRank("alice", "all", "tokens");

    expect(selectedKeys(1)).toEqual([
      "totalTokens",
      "totalCost",
    ]);
    expect(Object.keys(rank || {}).sort()).toEqual([
      "avatarUrl",
      "displayName",
      "rank",
      "totalCost",
      "totalTokens",
      "userId",
      "username",
    ]);
  });

  it("looks up all-time user rank usernames case-insensitively", async () => {
    mockState.pushAwaitedResult([
      {
        id: "user-imlunahey",
        username: "ImLunaHey",
        displayName: "Luna",
        avatarUrl: null,
      },
    ]);
    mockState.pushAwaitedResult([
      {
        totalTokens: 1200,
        totalCost: 12,
      },
    ]);
    mockState.pushAwaitedResult([{ count: 0 }]);

    const rank = await getUserRank("imlunahey", "all", "tokens");
    const sqlTexts = serializeSqlCalls();

    expect(rank).toMatchObject({
      rank: 1,
      username: "ImLunaHey",
      totalTokens: 1200,
    });
    expect(mockState.limitCalls[0]).toBe(2);
    expect(sqlTexts.some((text) =>
      text.toLowerCase().includes("lower(users.username) = imlunahey")
    )).toBe(true);
  });

  it("rejects ambiguous case-insensitive all-time user rank matches", async () => {
    mockState.pushAwaitedResult([
      {
        id: "user-imlunahey",
        username: "ImLunaHey",
        displayName: "Luna",
        avatarUrl: null,
      },
      {
        id: "user-imlunahey-duplicate",
        username: "imlunahey",
        displayName: "Luna Duplicate",
        avatarUrl: null,
      },
    ]);

    await expect(getUserRank("imlunahey", "all", "tokens")).rejects.toThrow(
      "Multiple users match username imlunahey case-insensitively"
    );
    expect(mockState.limitCalls[0]).toBe(2);
  });

  it("casts all-time cost at the full numeric(18,4) column precision so large totals don't overflow", async () => {
    // Regression: total_cost is numeric(18,4) (migration 0014); every cost cast must stay >= 18 wide or costs >= 1e8 overflow.
    // Width-based (not literal) so it also catches a re-narrowing to any precision below the column, e.g. DECIMAL(15,4).
    await getLeaderboardData("all", 1, 50, "cost");

    expectNoNarrowedCostCast(serializeSqlCalls());
  });
});

describe("all-time cost aggregation precision across query shapes (numeric overflow regression)", () => {
  // Complements the all-time-list check above with the search and user-rank
  // shapes that also cast submissions.total_cost (decimal(18,4)). Any cast
  // narrower than 18 overflows on costs >= the narrowed ceiling and 500s the
  // query; width-based so a re-narrowing to e.g. DECIMAL(15,4) is still caught.
  it("casts total_cost at full column precision in the all-time search list", async () => {
    mockState.pushAwaitedResult([]);
    mockState.pushAwaitedResult([{ count: 0 }]);
    mockState.pushAwaitedResult([
      { totalTokens: 0, totalCost: 0, uniqueUsers: 0 },
    ]);

    await getLeaderboardData("all", 1, 50, "cost", "ali");

    expectNoNarrowedCostCast(serializeSqlCalls());
  });

  it("casts total_cost at full column precision for all-time user rank", async () => {
    mockState.pushAwaitedResult([
      { id: "user-alice", username: "alice", displayName: "Alice", avatarUrl: null },
    ]);
    mockState.pushAwaitedResult([
      {
        totalTokens: 3000,
        totalCost: 40,
      },
    ]);
    mockState.pushAwaitedResult([{ count: 0 }]);

    await getUserRank("alice", "all", "cost");

    expectNoNarrowedCostCast(serializeSqlCalls());
  });
});
