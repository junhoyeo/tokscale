import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

import { expectNoNarrowedCostCast } from "../support/costCastWidths";

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
    or,
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

vi.mock("drizzle-orm", () => ({
  eq: mockState.eq,
  desc: mockState.desc,
  asc: mockState.asc,
  and: mockState.and,
  or: mockState.or,
  gte: mockState.gte,
  lte: mockState.lte,
  sql: mockState.sql,
}));

type ModuleExports = typeof import("../../src/lib/groups/getGroupLeaderboard");

let getGroupLeaderboardData: ModuleExports["getGroupLeaderboardData"];

function selectedKeys(callIndex: number): string[] {
  const calls = mockState.db.select.mock.calls as unknown as Array<
    [Record<string, unknown> | undefined]
  >;
  return Object.keys(calls[callIndex]?.[0] ?? {});
}

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
    },
    {
      userId: "user-bob",
      username: "bob",
      displayName: "Bob",
      avatarUrl: null,
      role: "member",
      tokens: 600,
      cost: 6,
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
    expect(leaderboard.stats).toEqual({
      totalTokens: 800,
      totalCost: 8,
      activeUsers: 2,
      totalMembers: 0,
    });
    expect(Object.keys(leaderboard.users[0]).sort()).toEqual([
      "avatarUrl",
      "displayName",
      "rank",
      "role",
      "totalCost",
      "totalTokens",
      "userId",
      "username",
    ]);
    expect(selectedKeys(1)).toEqual([
      "userId",
      "username",
      "displayName",
      "avatarUrl",
      "role",
      "tokens",
      "cost",
      "sourceBreakdown",
    ]);
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
      },
      {
        userId: "user-bob",
        username: "bob",
        displayName: "Bob",
        avatarUrl: null,
        role: "member",
        totalTokens: 100,
        totalCost: "3.0000",
      },
    ]);

    const leaderboard = await getGroupLeaderboardData("group-1", "all", 1, 50, "tokens");

    expect(mockState.fromCalls).toContain(mockState.tables.submissions);
    expect(mockState.orderByCalls[0]).toHaveLength(4);
    expect(mockState.asc).toHaveBeenCalledWith(mockState.tables.users.username);
    expect(mockState.asc).toHaveBeenCalledWith(mockState.tables.users.id);
    expect(leaderboard.users.map((user) => user.username)).toEqual(["alice", "bob"]);
    expect(selectedKeys(1)).toEqual([
      "userId",
      "username",
      "displayName",
      "avatarUrl",
      "role",
      "totalTokens",
      "totalCost",
    ]);
  });

  // submissions.total_cost is decimal(18,4); narrowing the cast to DECIMAL(12,4)
  // (max 99,999,999.9999) overflows for any row >= $100,000,000 and crashes the
  // all-time group leaderboard exactly like the global leaderboard.
  it("casts total_cost at full column precision for the all-time group leaderboard", async () => {
    mockState.setAllTimeRows([]);

    await getGroupLeaderboardData("group-1", "all", 1, 50, "cost");

    const sqlTexts = mockState.sql.mock.calls.map((call) => {
      const [strings, ...values] = call as [TemplateStringsArray, ...unknown[]];
      return Array.from(strings).reduce((text, part, index) => {
        const nextValue = index < values.length ? String(values[index]) : "";
        return `${text}${part}${nextValue}`;
      }, "");
    });

    expectNoNarrowedCostCast(sqlTexts);
  });

  it("ORs repeated directives in all-time group leaderboards", async () => {
    mockState.setAllTimeRows([]);

    await getGroupLeaderboardData(
      "group-1",
      "all",
      1,
      50,
      "tokens",
      "client:opencode client:claude model:gpt-5"
    );

    expect(mockState.or).toHaveBeenCalledTimes(2);
    expect(mockState.or.mock.calls.map((call) => call.length)).toEqual([2, 1]);
    // The rankable-user predicate is ANDed ahead of the directive groups: a
    // site-wide hide applies to group boards too.
    expect(mockState.and).toHaveBeenLastCalledWith(
      expect.anything(),
      mockState.or.mock.results[0]?.value,
      mockState.or.mock.results[1]?.value
    );
  });
});

describe("group period leaderboard directive scoping", () => {
  // One device, one day, two clients. The row total (1000/10) is the sum of
  // both, which is exactly what a filtered search must not credit.
  const mixedClientDay = {
    userId: "user-dana",
    username: "dana",
    displayName: "Dana",
    avatarUrl: null,
    role: "member",
    tokens: 1000,
    cost: 10,
    sourceBreakdown: {
      codex: {
        tokens: 300,
        cost: 3,
        models: {
          "gpt-5-codex": { tokens: 200, cost: 2 },
          "gpt-5-mini": { tokens: 100, cost: 1 },
        },
      },
      "claude-code": {
        tokens: 700,
        cost: 7,
        models: {
          "claude-opus-4": { tokens: 700, cost: 7 },
        },
      },
    },
  };

  function useWeekOf(rows: Array<Record<string, unknown>>) {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-03-07T18:45:00Z"));
    mockState.setPeriodRows(rows);
  }

  it("counts only the filtered client's share of a row shared with other clients", async () => {
    useWeekOf([mixedClientDay]);

    const leaderboard = await getGroupLeaderboardData(
      "group-1",
      "week",
      1,
      50,
      "tokens",
      "client:codex"
    );

    // 300/3, not the row's 1000/10: claude-code ran on the same day and
    // device but was not asked for.
    expect(leaderboard.users[0]).toMatchObject({
      username: "dana",
      role: "member",
      totalTokens: 300,
      totalCost: 3,
    });
    expect(leaderboard.stats).toMatchObject({
      totalTokens: 300,
      totalCost: 3,
      activeUsers: 1,
    });
  });

  it("counts only the filtered model's share, not every client in the row", async () => {
    useWeekOf([mixedClientDay]);

    const leaderboard = await getGroupLeaderboardData(
      "group-1",
      "week",
      1,
      50,
      "tokens",
      "model:claude-opus-4"
    );

    expect(leaderboard.users[0]).toMatchObject({
      username: "dana",
      totalTokens: 700,
      totalCost: 7,
    });
    expect(leaderboard.stats).toMatchObject({ totalTokens: 700, totalCost: 7 });
  });

  it("sums a model directive across every client that ran the model", async () => {
    useWeekOf([
      {
        ...mixedClientDay,
        sourceBreakdown: {
          codex: {
            tokens: 300,
            cost: 3,
            models: { "gpt-5-codex": { tokens: 300, cost: 3 } },
          },
          crush: {
            tokens: 700,
            cost: 7,
            models: { "gpt-5-codex": { tokens: 500, cost: 5 }, "gpt-4o": { tokens: 200, cost: 2 } },
          },
        },
      },
    ]);

    const leaderboard = await getGroupLeaderboardData(
      "group-1",
      "week",
      1,
      50,
      "tokens",
      "model:gpt-5-codex"
    );

    expect(leaderboard.users[0]).toMatchObject({ totalTokens: 800, totalCost: 8 });
  });

  it("reads client:x model:y as an intersection inside one client", async () => {
    useWeekOf([mixedClientDay]);

    // claude-opus-4 exists in the row, but under claude-code, not codex. The
    // union reading would have credited dana all 300 of her Codex tokens for
    // Opus work she never did there.
    const leaderboard = await getGroupLeaderboardData(
      "group-1",
      "week",
      1,
      50,
      "tokens",
      "client:codex model:claude-opus-4"
    );

    expect(leaderboard.users).toHaveLength(0);
    expect(leaderboard.stats).toMatchObject({
      totalTokens: 0,
      totalCost: 0,
      activeUsers: 0,
    });
  });

  it("keeps client matching on case-insensitive substrings", async () => {
    useWeekOf([mixedClientDay]);

    // `claude` is not a client id — it is a prefix of `claude-code`, and has
    // matched it since the directive shipped.
    const leaderboard = await getGroupLeaderboardData(
      "group-1",
      "week",
      1,
      50,
      "tokens",
      "client:CLAUDE"
    );

    expect(leaderboard.users[0]).toMatchObject({ totalTokens: 700, totalCost: 7 });
  });

  it("drops rows with no source breakdown when a directive is active", async () => {
    useWeekOf([
      mixedClientDay,
      {
        userId: "user-erin",
        username: "erin",
        displayName: "Erin",
        avatarUrl: null,
        role: "member",
        tokens: 5000,
        cost: 50,
        sourceBreakdown: null,
      },
    ]);

    const leaderboard = await getGroupLeaderboardData(
      "group-1",
      "week",
      1,
      50,
      "tokens",
      "client:codex"
    );

    expect(leaderboard.users.map((user) => user.username)).toEqual(["dana"]);
    expect(leaderboard.stats.totalTokens).toBe(300);
  });

  it("ranks on the filtered share, so a heavy unrelated client cannot buy a position", async () => {
    useWeekOf([
      mixedClientDay,
      {
        userId: "user-frank",
        username: "frank",
        displayName: "Frank",
        avatarUrl: null,
        role: "owner",
        tokens: 400,
        cost: 4,
        sourceBreakdown: {
          codex: {
            tokens: 400,
            cost: 4,
            models: { "gpt-5-codex": { tokens: 400, cost: 4 } },
          },
        },
      },
    ]);

    const leaderboard = await getGroupLeaderboardData(
      "group-1",
      "week",
      1,
      50,
      "tokens",
      "client:codex"
    );

    // dana's 1000-token row outweighs frank's 400 only because 700 of it is
    // claude-code. On Codex alone frank is ahead.
    expect(leaderboard.users.map((user) => user.username)).toEqual(["frank", "dana"]);
    expect(leaderboard.users.map((user) => user.totalTokens)).toEqual([400, 300]);
  });

  it("scopes every daily row a member contributes, not just the first", async () => {
    useWeekOf([
      mixedClientDay,
      {
        ...mixedClientDay,
        sourceBreakdown: {
          codex: {
            tokens: 50,
            cost: 0.5,
            models: { "gpt-5-codex": { tokens: 50, cost: 0.5 } },
          },
        },
        tokens: 50,
        cost: 0.5,
      },
    ]);

    const leaderboard = await getGroupLeaderboardData(
      "group-1",
      "week",
      1,
      50,
      "tokens",
      "client:codex"
    );

    // Both of dana's days fold into one entry, each already narrowed to Codex.
    expect(leaderboard.users).toHaveLength(1);
    expect(leaderboard.users[0]).toMatchObject({ totalTokens: 350, totalCost: 3.5 });
  });

  it("leaves the unfiltered path on the row totals", async () => {
    useWeekOf([mixedClientDay]);

    const leaderboard = await getGroupLeaderboardData("group-1", "week", 1, 50, "tokens");

    // No directive, so sourceBreakdown is never consulted and the whole row
    // counts, exactly as before.
    expect(leaderboard.users[0]).toMatchObject({
      username: "dana",
      totalTokens: 1000,
      totalCost: 10,
    });
    expect(leaderboard.stats).toMatchObject({
      totalTokens: 1000,
      totalCost: 10,
      activeUsers: 1,
    });
  });

  it("leaves a plain text search on the row totals", async () => {
    useWeekOf([mixedClientDay]);

    const leaderboard = await getGroupLeaderboardData("group-1", "week", 1, 50, "tokens", "dana");

    expect(leaderboard.users[0]).toMatchObject({ totalTokens: 1000, totalCost: 10 });
  });
});
