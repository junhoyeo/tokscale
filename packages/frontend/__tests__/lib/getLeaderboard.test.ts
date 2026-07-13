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
    execute: vi.fn((query: { strings?: string[]; values?: unknown[] }) => {
      const queryText = query.strings?.join("") ?? "";
      const queryValues = (values: unknown[]): string[] => values.flatMap((value) => {
        if (typeof value === "string") {
          return [value];
        }
        if (
          typeof value === "object"
          && value !== null
          && "values" in value
          && Array.isArray(value.values)
        ) {
          return queryValues(value.values);
        }
        return [];
      });
      const values = queryValues(query.values ?? []);
      const aggregated = Array.from(
        periodRows.reduce((usersById, row) => {
          const userId = String(row.userId);
          const existing = usersById.get(userId);
          if (existing) {
            existing.totalTokens += Number(row.tokens) || 0;
            existing.totalCost += Number(row.cost) || 0;
          } else {
            usersById.set(userId, {
              userId,
              username: String(row.username),
              displayName: (row.displayName as string | null) ?? null,
              avatarUrl: (row.avatarUrl as string | null) ?? null,
              totalTokens: Number(row.tokens) || 0,
              totalCost: Number(row.cost) || 0,
            });
          }
          return usersById;
        }, new Map<string, {
          userId: string;
          username: string;
          displayName: string | null;
          avatarUrl: string | null;
          totalTokens: number;
          totalCost: number;
        }>()).values()
      ).sort((left, right) =>
        right.totalTokens - left.totalTokens
        || right.totalCost - left.totalCost
        || left.username.localeCompare(right.username)
      ).map((user, index) => ({ ...user, rank: index + 1 }));

      if (!queryText.includes("json_agg")) {
        const username = values.find((value) =>
          !/^\d{4}-\d{2}-\d{2}$/.test(value)
        );
        return Promise.resolve(aggregated.filter((user) =>
          user.username.toLowerCase() === String(username).toLowerCase()
        ));
      }

      const searchPattern = values.find((value) =>
        value.startsWith("%") && value.endsWith("%")
      );
      const search = searchPattern ? String(searchPattern).slice(1, -1).toLowerCase() : "";
      const users = search
        ? aggregated.filter((user) =>
            user.username.toLowerCase().includes(search)
            || user.displayName?.toLowerCase().includes(search)
          )
        : aggregated;

      return Promise.resolve([{
        users,
        totalUsers: users.length,
        totalTokens: aggregated.reduce((sum, user) => sum + user.totalTokens, 0),
        totalCost: aggregated.reduce((sum, user) => sum + user.totalCost, 0),
        uniqueUsers: aggregated.length,
      }]);
    }),
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
      db.execute.mockClear();
      eq.mockClear();
      desc.mockClear();
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
  gte: mockState.gte,
  lte: mockState.lte,
  sql: mockState.sql,
}));

type ModuleExports = typeof import("../../src/lib/leaderboard/getLeaderboard");

let getLeaderboardData: ModuleExports["getLeaderboardData"];
let getUserRank: ModuleExports["getUserRank"];

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

    const sqlTexts = serializeSqlCalls();
    expect(sqlTexts.some((text) => text.includes("FROM daily_breakdown"))).toBe(true);
    expect(sqlTexts.some((text) => text.includes("2026-03-01") && text.includes("2026-03-07"))).toBe(true);
    expect(sqlTexts.some((text) => text.includes("ROW_NUMBER() OVER"))).toBe(true);
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
  });

  it("uses the current month for the month leaderboard range", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-03-07T18:45:00Z"));
    mockState.setPeriodRows(rows);

    const leaderboard = await getLeaderboardData("month", 1, 50, "tokens");

    const sqlTexts = serializeSqlCalls();
    expect(sqlTexts.some((text) => text.includes("2026-03-01") && text.includes("2026-03-07"))).toBe(true);
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
