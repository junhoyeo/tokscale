import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

const mockState = vi.hoisted(() => {
  const awaitedResults: unknown[] = [];

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
        limit: vi.fn(() => builder),
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
});
