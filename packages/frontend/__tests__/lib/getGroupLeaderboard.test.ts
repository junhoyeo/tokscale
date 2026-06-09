import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

const mockState = vi.hoisted(() => {
  const periodRows: Array<Record<string, unknown>> = [];
  const allTimeRows: Array<Record<string, unknown>> = [];
  const statsRows: Array<Record<string, unknown>> = [];
  const countRows: Array<Record<string, unknown>> = [];
  const fromCalls: unknown[] = [];
  const orderByCalls: unknown[][] = [];

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
      cliVersion: "submissions.cliVersion",
      schemaVersion: "submissions.schemaVersion",
    },
    dailyBreakdown: {
      submissionId: "dailyBreakdown.submissionId",
      date: "dailyBreakdown.date",
      tokens: "dailyBreakdown.tokens",
      cost: "dailyBreakdown.cost",
    },
    groupMembers: {
      groupId: "groupMembers.groupId",
      userId: "groupMembers.userId",
      role: "groupMembers.role",
    },
  };

  const eq = vi.fn(() => "eq");
  const desc = vi.fn(() => "desc");
  const asc = vi.fn(() => "asc");
  const and = vi.fn(() => "and");
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

  function nextRows(table: unknown) {
    if (table === tables.dailyBreakdown) {
      return [...periodRows];
    }
    if (table === tables.submissions) {
      return [...allTimeRows];
    }
    if (table === tables.groupMembers) {
      return countRows.shift() ? [...countRows] : [];
    }
    return statsRows.shift() ? [...statsRows] : [];
  }

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
        leftJoin: vi.fn(() => builder),
        where: vi.fn(() => builder),
        groupBy: vi.fn(() => builder),
        orderBy: vi.fn((...args: unknown[]) => {
          orderByCalls.push(args);
          return builder;
        }),
        limit: vi.fn(() => builder),
        offset: vi.fn(() => builder),
        then: (resolve: (value: unknown) => unknown) => resolve(nextRows(selectedTable)),
      };

      return builder;
    }),
  };

  return {
    db,
    tables,
    fromCalls,
    orderByCalls,
    eq,
    desc,
    asc,
    and,
    gte,
    lte,
    sql,
    reset() {
      periodRows.length = 0;
      allTimeRows.length = 0;
      statsRows.length = 0;
      countRows.length = 0;
      fromCalls.length = 0;
      orderByCalls.length = 0;
      db.select.mockClear();
      eq.mockClear();
      desc.mockClear();
      asc.mockClear();
      and.mockClear();
      gte.mockClear();
      lte.mockClear();
      sql.mockClear();
      sql.raw.mockClear();
    },
    setPeriodRows(rows: Array<Record<string, unknown>>) {
      periodRows.length = 0;
      periodRows.push(...rows);
    },
    setAllTimeRows(rows: Array<Record<string, unknown>>) {
      allTimeRows.length = 0;
      allTimeRows.push(...rows);
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
  groupMembers: mockState.tables.groupMembers,
}));

vi.mock("@/lib/submissionFreshness", async () =>
  import("../../src/lib/submissionFreshness")
);

vi.mock("drizzle-orm", () => ({
  eq: mockState.eq,
  desc: mockState.desc,
  asc: mockState.asc,
  and: mockState.and,
  gte: mockState.gte,
  lte: mockState.lte,
  sql: mockState.sql,
}));

type ModuleExports = typeof import("../../src/lib/groups/getGroupLeaderboard");

let getGroupLeaderboardData: ModuleExports["getGroupLeaderboardData"];

beforeAll(async () => {
  const groupLeaderboardModule = await import("../../src/lib/groups/getGroupLeaderboard");
  getGroupLeaderboardData = groupLeaderboardModule.getGroupLeaderboardData;
});

beforeEach(() => {
  mockState.reset();
});

afterEach(() => {
  vi.useRealTimers();
});

describe("group leaderboard data", () => {
  const rows = [
    {
      userId: "user-alice",
      username: "alice",
      displayName: "Alice",
      avatarUrl: null,
      role: "owner",
      tokens: 200,
      cost: 2,
      updatedAt: "2026-03-07T11:00:00.000Z",
      cliVersion: "1.5.0",
      schemaVersion: 1,
    },
    {
      userId: "user-bob",
      username: "bob",
      displayName: "Bob",
      avatarUrl: null,
      role: "member",
      tokens: 600,
      cost: 6,
      updatedAt: "2026-03-06T11:00:00.000Z",
      cliVersion: "1.5.0",
      schemaVersion: 1,
    },
  ];

  it("builds period rankings from daily rows scoped through group membership", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-03-07T18:45:00Z"));
    mockState.setPeriodRows(rows);

    const leaderboard = await getGroupLeaderboardData("group-1", "week", 1, 50, "tokens");

    expect(mockState.fromCalls).toContain(mockState.tables.dailyBreakdown);
    expect(mockState.gte).toHaveBeenCalledWith(
      mockState.tables.dailyBreakdown.date,
      "2026-03-01"
    );
    expect(mockState.lte).toHaveBeenCalledWith(
      mockState.tables.dailyBreakdown.date,
      "2026-03-07"
    );
    expect(mockState.eq).toHaveBeenCalledWith(
      mockState.tables.groupMembers.groupId,
      "group-1"
    );
    expect(leaderboard.users.map((user) => user.username)).toEqual(["bob", "alice"]);
    expect(leaderboard.users[0]).toMatchObject({
      rank: 1,
      role: "member",
      totalTokens: 600,
    });
    expect(leaderboard.stats).toMatchObject({
      totalTokens: 800,
      totalCost: 8,
      activeUsers: 2,
    });
  });

  it("filters search results after computing scoped ranks", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-03-07T18:45:00Z"));
    mockState.setPeriodRows(rows);

    const leaderboard = await getGroupLeaderboardData("group-1", "week", 1, 50, "tokens", "ali");

    expect(leaderboard.users).toHaveLength(1);
    expect(leaderboard.users[0]).toMatchObject({
      rank: 2,
      username: "alice",
    });
    expect(leaderboard.pagination.totalUsers).toBe(1);
  });

  it("adds deterministic SQL tie-breakers before assigning all-time ranks", async () => {
    mockState.setAllTimeRows([
      {
        userId: "user-alice",
        username: "alice",
        displayName: "Alice",
        avatarUrl: null,
        role: "member",
        totalTokens: 100,
        totalCost: "3.0000",
        submissionCount: 1,
        lastSubmission: "2026-03-07T11:00:00.000Z",
        cliVersion: "1.5.0",
        schemaVersion: 1,
      },
      {
        userId: "user-bob",
        username: "bob",
        displayName: "Bob",
        avatarUrl: null,
        role: "member",
        totalTokens: 100,
        totalCost: "3.0000",
        submissionCount: 1,
        lastSubmission: "2026-03-07T11:00:00.000Z",
        cliVersion: "1.5.0",
        schemaVersion: 1,
      },
    ]);

    const leaderboard = await getGroupLeaderboardData("group-1", "all", 1, 50, "tokens");

    expect(mockState.fromCalls).toContain(mockState.tables.submissions);
    expect(mockState.orderByCalls[0]).toHaveLength(4);
    expect(mockState.asc).toHaveBeenCalledWith(mockState.tables.users.username);
    expect(mockState.asc).toHaveBeenCalledWith(mockState.tables.users.id);
    expect(mockState.sql).toHaveBeenCalledWith(
      expect.arrayContaining(["(\n        SELECT s2.cli_version FROM submissions s2\n        WHERE s2.user_id = ", "\n        ORDER BY s2.updated_at DESC, s2.id DESC LIMIT 1\n      )"]),
      mockState.tables.users.id
    );
    expect(leaderboard.users.map((user) => user.username)).toEqual(["alice", "bob"]);
  });
});
