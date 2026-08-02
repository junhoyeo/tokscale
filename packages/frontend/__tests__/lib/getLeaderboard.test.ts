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

  it("drops hidden users from the rankings but keeps them in the totals", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-03-07T18:45:00Z"));
    // carol outranks everyone on raw tokens but is hidden.
    mockState.setPeriodRows([
      ...rows,
      {
        userId: "user-carol",
        username: "carol",
        displayName: "Carol",
        avatarUrl: null,
        tokens: 9_000_000,
        cost: 500,
        leaderboardHidden: true,
      },
    ]);

    const leaderboard = await getLeaderboardData("week", 1, 50, "tokens");

    expect(leaderboard.users.map((user) => user.username)).toEqual(["bob", "alice"]);

    // Dense ranks: bob takes 1st rather than 2nd. Leaving carol's position
    // vacant would advertise that someone was removed.
    expect(leaderboard.users.map((user) => user.rank)).toEqual([1, 2]);

    // ...but the totals still count carol. Hiding withdraws an account from
    // the competition; it does not retract the usage from the site figures.
    expect(leaderboard.stats.totalTokens).toBe(9_001_250);
    expect(leaderboard.stats.uniqueUsers).toBe(3);

    // Pagination must track the rankable set, not the totals, or a trailing
    // page renders empty.
    expect(leaderboard.pagination.totalUsers).toBe(2);
  });

  it("reports no rank for a hidden user rather than a phantom position", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-03-07T18:45:00Z"));
    mockState.setPeriodRows([
      ...rows,
      {
        userId: "user-carol",
        username: "carol",
        displayName: "Carol",
        avatarUrl: null,
        tokens: 9_000_000,
        cost: 500,
        leaderboardHidden: true,
      },
    ]);

    // A rank would not correspond to any row on the board she is absent from.
    await expect(getUserRank("carol", "week", "tokens")).resolves.toBeNull();
  });

  it("does not let a hidden user above you inflate your own rank", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-03-07T18:45:00Z"));
    mockState.setPeriodRows([
      ...rows,
      {
        userId: "user-carol",
        username: "carol",
        displayName: "Carol",
        avatarUrl: null,
        tokens: 9_000_000,
        cost: 500,
        leaderboardHidden: true,
      },
    ]);

    // bob is 2nd by raw tokens but 1st among rankable users.
    await expect(getUserRank("bob", "week", "tokens")).resolves.toMatchObject({
      username: "bob",
      rank: 1,
    });
  });

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
      // Selected, not filtered in SQL: the period totals are summed from these
      // same rows and must still include hidden users, so the exclusion is
      // applied after aggregation rather than in the WHERE clause.
      "leaderboardHidden",
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

describe("period leaderboard directive scoping", () => {
  // One device, one day, two clients. The row total (1000/10) is the sum of
  // both, which is exactly what a filtered search must not credit.
  const mixedClientDay = {
    userId: "user-dana",
    username: "dana",
    displayName: "Dana",
    avatarUrl: null,
    tokens: 1000,
    cost: 10,
    leaderboardHidden: false,
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

    const leaderboard = await getLeaderboardData("week", 1, 50, "tokens", "client:codex");

    // 300/3, not the row's 1000/10: claude-code ran on the same day and
    // device but was not asked for.
    expect(leaderboard.users[0]).toMatchObject({
      username: "dana",
      totalTokens: 300,
      totalCost: 3,
    });
    expect(leaderboard.stats).toMatchObject({
      totalTokens: 300,
      totalCost: 3,
      uniqueUsers: 1,
    });
  });

  it("counts only the filtered model's share, not every client in the row", async () => {
    useWeekOf([mixedClientDay]);

    const leaderboard = await getLeaderboardData("week", 1, 50, "tokens", "model:claude-opus-4");

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

    const leaderboard = await getLeaderboardData("week", 1, 50, "tokens", "model:gpt-5-codex");

    expect(leaderboard.users[0]).toMatchObject({ totalTokens: 800, totalCost: 8 });
  });

  it("reads client:x model:y as an intersection inside one client", async () => {
    useWeekOf([mixedClientDay]);

    // claude-opus-4 exists in the row, but under claude-code, not codex. The
    // union reading would have credited dana all 300 of her Codex tokens for
    // Opus work she never did there.
    const leaderboard = await getLeaderboardData(
      "week",
      1,
      50,
      "tokens",
      "client:codex model:claude-opus-4"
    );

    expect(leaderboard.users).toHaveLength(0);
    expect(leaderboard.stats).toMatchObject({ totalTokens: 0, totalCost: 0, uniqueUsers: 0 });
  });

  it("keeps client matching on case-insensitive substrings", async () => {
    useWeekOf([mixedClientDay]);

    // `claude` is not a client id — it is a prefix of `claude-code`, and has
    // matched it since the directive shipped.
    const leaderboard = await getLeaderboardData("week", 1, 50, "tokens", "client:CLAUDE");

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
        tokens: 5000,
        cost: 50,
        leaderboardHidden: false,
        sourceBreakdown: null,
      },
    ]);

    const leaderboard = await getLeaderboardData("week", 1, 50, "tokens", "client:codex");

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
        tokens: 400,
        cost: 4,
        leaderboardHidden: false,
        sourceBreakdown: {
          codex: {
            tokens: 400,
            cost: 4,
            models: { "gpt-5-codex": { tokens: 400, cost: 4 } },
          },
        },
      },
    ]);

    const leaderboard = await getLeaderboardData("week", 1, 50, "tokens", "client:codex");

    // dana's 1000-token row outweighs frank's 400 only because 700 of it is
    // claude-code. On Codex alone frank is ahead.
    expect(leaderboard.users.map((user) => user.username)).toEqual(["frank", "dana"]);
    expect(leaderboard.users.map((user) => user.totalTokens)).toEqual([400, 300]);
  });

  it("still counts a hidden user's filtered share in the period totals", async () => {
    useWeekOf([mixedClientDay, { ...mixedClientDay, userId: "user-gil", username: "gil", leaderboardHidden: true }]);

    const leaderboard = await getLeaderboardData("week", 1, 50, "tokens", "client:codex");

    expect(leaderboard.users.map((user) => user.username)).toEqual(["dana"]);
    // Both users' Codex share, neither user's claude-code share.
    expect(leaderboard.stats).toMatchObject({ totalTokens: 600, uniqueUsers: 2 });
    expect(leaderboard.pagination.totalUsers).toBe(1);
  });

  it("leaves the unfiltered path on the row totals", async () => {
    useWeekOf([mixedClientDay]);

    const leaderboard = await getLeaderboardData("week", 1, 50, "tokens");

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
      uniqueUsers: 1,
    });
  });

  it("leaves a plain text search on the row totals", async () => {
    useWeekOf([mixedClientDay]);

    const leaderboard = await getLeaderboardData("week", 1, 50, "tokens", "dana");

    expect(leaderboard.users[0]).toMatchObject({ totalTokens: 1000, totalCost: 10 });
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
    // The rankable-user predicate is ANDed ahead of the directive groups, so
    // a search can never surface a hidden user.
    expect(mockState.and).toHaveBeenCalledWith(
      expect.anything(),
      mockState.or.mock.results[0]?.value,
      mockState.or.mock.results[1]?.value
    );

    const boundValues = mockState.sql.mock.calls.flatMap(([, ...values]) => values);
    expect(boundValues).toContain("%gpt\\_5%");
  });
});
