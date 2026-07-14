import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

const mockState = vi.hoisted(() => {
  const periodRows: Array<Record<string, unknown>> = [];
  const fromCalls: unknown[] = [];

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
  const or = vi.fn((...conditions: unknown[]) => ({ kind: "or", conditions }));
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
      let selectedTable: unknown;
      const builder = {
        from: vi.fn((table: unknown) => {
          selectedTable = table;
          fromCalls.push(table);
          return builder;
        }),
        innerJoin: vi.fn(() => builder),
        where: vi.fn(() => builder),
        groupBy: vi.fn(() => builder),
        orderBy: vi.fn(() => builder),
        limit: vi.fn(() => builder),
        offset: vi.fn(() => builder),
        as: vi.fn(() => ({
          rank: "ranked.rank",
          userId: "ranked.userId",
          username: "ranked.username",
          displayName: "ranked.displayName",
          avatarUrl: "ranked.avatarUrl",
          totalTokens: "ranked.totalTokens",
          totalCost: "ranked.totalCost",
        })),
        then: (resolve: (value: unknown) => unknown) => {
          if (selectedTable === tables.dailyBreakdown) {
            return resolve([...periodRows]);
          }
          return resolve([]);
        },
      };

      return builder;
    }),
  };

  return {
    db,
    tables,
    fromCalls,
    eq,
    desc,
    and,
    or,
    gte,
    lte,
    sql,
    reset() {
      periodRows.length = 0;
      fromCalls.length = 0;
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
    setPeriodRows(rows: Array<Record<string, unknown>>) {
      periodRows.length = 0;
      periodRows.push(...rows);
    },
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

describe("period leaderboard data", () => {
  const rows = [
    {
      userId: "user-alice",
      username: "alice",
      displayName: "Alice",
      avatarUrl: null,
      tokens: 100,
      cost: 1.25,
    },
    {
      userId: "user-alice",
      username: "alice",
      displayName: "Alice",
      avatarUrl: null,
      tokens: 150,
      cost: 1.75,
    },
    {
      userId: "user-bob",
      username: "bob",
      displayName: "Bob",
      avatarUrl: null,
      tokens: 1000,
      cost: 9.5,
    },
  ];

  it("builds the week leaderboard from daily rows", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-03-07T18:45:00Z"));
    mockState.setPeriodRows(rows);

    const leaderboard = await getLeaderboardData("week", 1, 50, "tokens");

    expect(mockState.fromCalls[0]).toBe(mockState.tables.dailyBreakdown);
    expect(mockState.gte).toHaveBeenCalledWith(
      mockState.tables.dailyBreakdown.date,
      "2026-03-01"
    );
    expect(mockState.lte).toHaveBeenCalledWith(
      mockState.tables.dailyBreakdown.date,
      "2026-03-07"
    );
    expect(leaderboard.users).toHaveLength(2);
    expect(leaderboard.users[0]).toMatchObject({
      rank: 1,
      username: "bob",
      totalTokens: 1000,
      totalCost: 9.5,
    });
    expect(leaderboard.users[1]).toMatchObject({
      rank: 2,
      username: "alice",
      totalTokens: 250,
      totalCost: 3,
    });
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
      totalTokens: 1250,
      totalCost: 12.5,
      uniqueUsers: 2,
    });
    expect(selectedKeys(0)).toEqual([
      "userId",
      "username",
      "displayName",
      "avatarUrl",
      "tokens",
      "cost",
      "sourceBreakdown",
    ]);
  });

  it("uses the current month for the month leaderboard range", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-03-07T18:45:00Z"));
    mockState.setPeriodRows(rows);

    const leaderboard = await getLeaderboardData("month", 1, 50, "tokens");

    expect(mockState.fromCalls[0]).toBe(mockState.tables.dailyBreakdown);
    expect(mockState.gte).toHaveBeenCalledWith(
      mockState.tables.dailyBreakdown.date,
      "2026-03-01"
    );
    expect(mockState.lte).toHaveBeenCalledWith(
      mockState.tables.dailyBreakdown.date,
      "2026-03-07"
    );
    expect(leaderboard.users[1]).toMatchObject({
      username: "alice",
      totalTokens: 250,
      totalCost: 3,
    });
  });

  it("filters period leaderboards by username while preserving each user's true rank", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-03-07T18:45:00Z"));
    mockState.setPeriodRows(rows);

    const leaderboard = await getLeaderboardData("week", 1, 50, "tokens", "ali");

    expect(leaderboard.users).toHaveLength(1);
    expect(leaderboard.users[0]).toMatchObject({
      rank: 2,
      username: "alice",
      totalTokens: 250,
      totalCost: 3,
    });
    expect(leaderboard.pagination).toMatchObject({
      totalUsers: 1,
      totalPages: 1,
      hasNext: false,
      hasPrev: false,
    });
    expect(leaderboard.stats).toMatchObject({
      totalTokens: 1250,
      totalCost: 12.5,
      uniqueUsers: 2,
    });
  });

  it("uses the same daily totals when computing week rank", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-03-07T18:45:00Z"));
    mockState.setPeriodRows(rows);

    const rank = await getUserRank("alice", "week", "tokens");

    expect(mockState.fromCalls[0]).toBe(mockState.tables.dailyBreakdown);
    expect(rank).toMatchObject({
      rank: 2,
      username: "alice",
      totalTokens: 250,
      totalCost: 3,
    });
  });

  it("matches period user rank usernames case-insensitively", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-03-07T18:45:00Z"));
    mockState.setPeriodRows(rows);

    const rank = await getUserRank("ALICE", "week", "tokens");

    expect(rank).toMatchObject({
      rank: 2,
      username: "alice",
      totalTokens: 250,
      totalCost: 3,
    });
  });

  it("rejects ambiguous case-insensitive period user rank matches", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-03-07T18:45:00Z"));
    mockState.setPeriodRows([
      ...rows,
      {
        userId: "user-alice-duplicate",
        username: "ALICE",
        displayName: "Alice Duplicate",
        avatarUrl: null,
        tokens: 50,
        cost: 0.5,
      },
    ]);

    await expect(getUserRank("alice", "week", "tokens")).rejects.toThrow(
      "Multiple users match username alice case-insensitively"
    );
  });
});

describe("all-time leaderboard directives", () => {
  it("ORs repeated directives within each type before combining types", async () => {
    await getLeaderboardData(
      "all",
      1,
      50,
      "tokens",
      "client:opencode client:claude model:gpt_5"
    );

    expect(mockState.or).toHaveBeenCalledTimes(2);
    expect(mockState.or.mock.calls.map((call) => call.length)).toEqual([2, 1]);
    expect(mockState.and).toHaveBeenCalledWith(
      mockState.or.mock.results[0]?.value,
      mockState.or.mock.results[1]?.value
    );

    const boundValues = mockState.sql.mock.calls.flatMap(([, ...values]) => values);
    expect(boundValues).toContain("%gpt\\_5%");
  });
});
