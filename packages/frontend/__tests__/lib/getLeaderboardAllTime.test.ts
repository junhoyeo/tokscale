import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

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

describe("all-time leaderboard freshness queries", () => {
  it("uses latest-row scalar subqueries instead of MAX(cliVersion/schemaVersion)", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-03-12T18:45:00Z"));

    mockState.pushAwaitedResult([
      {
        rank: 1,
        userId: "user-alice",
        username: "alice",
        displayName: "Alice",
        avatarUrl: null,
        totalTokens: 3000,
        totalCost: 30,
        submissionCount: 2,
        lastSubmission: "2026-03-12T09:00:00.000Z",
        cliVersion: "1.9.0",
        schemaVersion: 1,
      },
    ]);
    mockState.pushAwaitedResult([
      {
        totalTokens: 3000,
        totalCost: 30,
        totalSubmissions: 2,
        uniqueUsers: 1,
      },
    ]);

    const leaderboard = await getLeaderboardData("all", 1, 50, "tokens");
    const sqlTexts = serializeSqlCalls();

    expect(sqlTexts.some((text) =>
      text.includes("SELECT s2.cli_version FROM submissions s2")
        && text.includes("ORDER BY s2.updated_at DESC LIMIT 1")
    )).toBe(true);
    expect(sqlTexts.some((text) =>
      text.includes("SELECT s2.schema_version FROM submissions s2")
        && text.includes("ORDER BY s2.updated_at DESC LIMIT 1")
    )).toBe(true);
    expect(sqlTexts.some((text) =>
      text.includes("MAX(") && text.includes("submissions.cliVersion")
    )).toBe(false);
    expect(sqlTexts.some((text) =>
      text.includes("MAX(") && text.includes("submissions.schemaVersion")
    )).toBe(false);
    expect(leaderboard.users[0]).toMatchObject({
      rank: 1,
      username: "alice",
      lastSubmission: "2026-03-12T09:00:00.000Z",
      submissionFreshness: {
        lastUpdated: "2026-03-12T09:00:00.000Z",
        cliVersion: "1.9.0",
        schemaVersion: 1,
        isStale: false,
      },
    });
  });

  it("uses latest-row scalar subqueries for all-time user rank metadata", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-03-12T18:45:00Z"));

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
        submissionCount: 2,
        lastSubmission: "2026-03-12T09:00:00.000Z",
        cliVersion: "1.9.0",
        schemaVersion: 1,
      },
    ]);
    mockState.pushAwaitedResult([
      {
        count: 0,
      },
    ]);

    const rank = await getUserRank("alice", "all", "tokens");
    const sqlTexts = serializeSqlCalls();

    expect(sqlTexts.some((text) =>
      text.includes("SELECT s2.cli_version FROM submissions s2")
        && text.includes("WHERE s2.user_id = user-alice")
    )).toBe(true);
    expect(sqlTexts.some((text) =>
      text.includes("SELECT s2.schema_version FROM submissions s2")
        && text.includes("WHERE s2.user_id = user-alice")
    )).toBe(true);
    expect(sqlTexts.some((text) =>
      text.includes("MAX(") && text.includes("submissions.cliVersion")
    )).toBe(false);
    expect(sqlTexts.some((text) =>
      text.includes("MAX(") && text.includes("submissions.schemaVersion")
    )).toBe(false);
    expect(rank).toMatchObject({
      rank: 1,
      username: "alice",
      totalTokens: 3000,
      totalCost: 30,
      submissionCount: 2,
      lastSubmission: "2026-03-12T09:00:00.000Z",
      submissionFreshness: {
        lastUpdated: "2026-03-12T09:00:00.000Z",
        cliVersion: "1.9.0",
        schemaVersion: 1,
        isStale: false,
      },
    });
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
        submissionCount: 1,
        lastSubmission: "2026-03-12T09:00:00.000Z",
        cliVersion: "1.9.0",
        schemaVersion: 1,
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
});
