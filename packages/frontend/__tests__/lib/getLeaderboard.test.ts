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
      submitCount: "submissions.submitCount",
      updatedAt: "submissions.updatedAt",
      totalTokens: "submissions.totalTokens",
      totalCost: "submissions.totalCost",
      cliVersion: "submissions.cliVersion",
      schemaVersion: "submissions.schemaVersion",
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
  const gte = vi.fn(() => "gte");
  const lte = vi.fn(() => "lte");
  const sql = Object.assign(
    () => ({
      as: () => ({}),
    }),
    {
      raw: vi.fn(),
    }
  );

  const db = {
    select: vi.fn(() => {
      const builder = {
        from: vi.fn((table: unknown) => {
          fromCalls.push(table);
          return builder;
        }),
        innerJoin: vi.fn(() => builder),
        where: vi.fn(async () => [...periodRows]),
        groupBy: vi.fn(() => builder),
        orderBy: vi.fn(() => builder),
        limit: vi.fn(() => builder),
        offset: vi.fn(() => builder),
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
      gte.mockClear();
      lte.mockClear();
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

vi.mock("@/lib/submissionFreshness", async () =>
  import("../../src/lib/submissionFreshness")
);

vi.mock("drizzle-orm", () => ({
  eq: mockState.eq,
  desc: mockState.desc,
  and: mockState.and,
  gte: mockState.gte,
  lte: mockState.lte,
  sql: mockState.sql,
}));

type ModuleExports = typeof import("../../src/lib/leaderboard/getLeaderboard");

let getLeaderboardData: ModuleExports["getLeaderboardData"];
let getUserRank: ModuleExports["getUserRank"];

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
      updatedAt: "2026-03-07T11:00:00.000Z",
      cliVersion: "1.5.0",
      schemaVersion: 1,
    },
    {
      userId: "user-alice",
      username: "alice",
      displayName: "Alice",
      avatarUrl: null,
      tokens: 150,
      cost: 1.75,
      updatedAt: "2026-03-07T11:00:00.000Z",
      cliVersion: "1.5.0",
      schemaVersion: 1,
    },
    {
      userId: "user-bob",
      username: "bob",
      displayName: "Bob",
      avatarUrl: null,
      tokens: 1000,
      cost: 9.5,
      updatedAt: "2026-01-15T09:00:00.000Z",
      cliVersion: "1.3.0",
      schemaVersion: 0,
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
      submissionFreshness: {
        lastUpdated: "2026-01-15T09:00:00.000Z",
        cliVersion: "1.3.0",
        schemaVersion: 0,
        isStale: true,
      },
    });
    expect(leaderboard.users[1]).toMatchObject({
      rank: 2,
      username: "alice",
      totalTokens: 250,
      totalCost: 3,
      submissionCount: null,
      submissionFreshness: {
        lastUpdated: "2026-03-07T11:00:00.000Z",
        cliVersion: "1.5.0",
        schemaVersion: 1,
        isStale: false,
      },
    });
    expect(leaderboard.stats).toMatchObject({
      totalTokens: 1250,
      totalCost: 12.5,
      totalSubmissions: null,
      uniqueUsers: 2,
    });
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
      submissionCount: null,
      lastSubmission: "2026-03-07T11:00:00.000Z",
      submissionFreshness: {
        lastUpdated: "2026-03-07T11:00:00.000Z",
        cliVersion: "1.5.0",
        schemaVersion: 1,
        isStale: false,
      },
    });
  });
});
