import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

const mockState = vi.hoisted(() => {
  const selectResults: Array<Array<Record<string, unknown>>> = [];
  const executeResults: Array<Array<Record<string, unknown>>> = [];

  const tables = {
    users: {
      id: "users.id",
      username: "users.username",
      displayName: "users.displayName",
      avatarUrl: "users.avatarUrl",
    },
    submissions: {
      userId: "submissions.userId",
      totalTokens: "submissions.totalTokens",
      totalCost: "submissions.totalCost",
      submitCount: "submissions.submitCount",
      updatedAt: "submissions.updatedAt",
    },
  };

  const eq = vi.fn(() => "eq");
  const sql = vi.fn((strings: TemplateStringsArray, ...values: unknown[]) => ({
    strings: Array.from(strings),
    values,
  }));

  const groupBy = vi.fn(() => builder);
  const limit = vi.fn(() => builder);
  const where = vi.fn(() => builder);
  const leftJoin = vi.fn(() => builder);
  const from = vi.fn(() => builder);
  const select = vi.fn(() => builder);

  const builder = {
    from,
    leftJoin,
    where,
    groupBy,
    limit,
    then: (resolve: (value: unknown) => unknown) => resolve(selectResults.shift() ?? []),
  };

  return {
    db: {
      select,
      execute: vi.fn(async () => executeResults.shift() ?? []),
    },
    tables,
    eq,
    sql,
    groupBy,
    reset() {
      selectResults.length = 0;
      executeResults.length = 0;
      select.mockClear();
      from.mockClear();
      leftJoin.mockClear();
      where.mockClear();
      groupBy.mockClear();
      limit.mockClear();
      this.db.execute.mockClear();
      eq.mockClear();
      sql.mockClear();
    },
    pushSelectResult(rows: Array<Record<string, unknown>>) {
      selectResults.push(rows);
    },
    pushExecuteResult(rows: Array<Record<string, unknown>>) {
      executeResults.push(rows);
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
}));

vi.mock("drizzle-orm", () => ({
  eq: mockState.eq,
  sql: mockState.sql,
}));

type ModuleExports = typeof import("../../src/lib/embed/getUserEmbedStats");

let getUserEmbedStats: ModuleExports["getUserEmbedStats"];

function serializeSqlCalls(): string[] {
  return mockState.sql.mock.calls.map((call) => {
    const [strings, ...values] = call as [TemplateStringsArray, ...unknown[]];
    return Array.from(strings).reduce((text, part, index) => {
      const nextValue = index < values.length ? String(values[index]) : "";
      return `${text}${part}${nextValue}`;
    }, "");
  });
}

beforeAll(async () => {
  const module = await import("../../src/lib/embed/getUserEmbedStats");
  getUserEmbedStats = module.getUserEmbedStats;
});

beforeEach(() => {
  mockState.reset();
});

describe("getUserEmbedStats", () => {
  it("aggregates totals across multiple submission rows", async () => {
    mockState.pushSelectResult([
      {
        id: "user-1",
        username: "alice",
        displayName: "Alice",
        avatarUrl: null,
        totalTokens: 300,
        totalCost: 3.5,
        submissionCount: 5,
        updatedAt: new Date("2026-03-29T10:00:00.000Z"),
      },
    ]);
    mockState.pushExecuteResult([{ rank: 2 }]);

    const result = await getUserEmbedStats("alice", "tokens");
    const sqlTexts = serializeSqlCalls();

    expect(mockState.groupBy).toHaveBeenCalled();
    expect(sqlTexts.some((text) =>
      text.includes("WITH user_totals AS") && text.includes("GROUP BY user_id")
    )).toBe(true);
    expect(result).toEqual({
      user: {
        id: "user-1",
        username: "alice",
        displayName: "Alice",
        avatarUrl: null,
      },
      stats: {
        totalTokens: 300,
        totalCost: 3.5,
        submissionCount: 5,
        rank: 2,
        updatedAt: "2026-03-29T10:00:00.000Z",
      },
    });
  });
});
