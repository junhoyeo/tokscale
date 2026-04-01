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

  const db = {
    select: vi.fn(() => {
      const builder = {
        from: vi.fn(() => builder),
        leftJoin: vi.fn(() => builder),
        where: vi.fn(() => builder),
        groupBy: vi.fn(() => builder),
        limit: vi.fn(() => builder),
        then: (resolve: (value: unknown) => unknown) =>
          resolve(selectResults.shift() ?? []),
      };

      return builder;
    }),
    execute: vi.fn(async () => executeResults.shift() ?? []),
  };

  const eq = vi.fn(() => "eq");
  const sql = Object.assign(
    () => ({
      as: () => ({}),
    }),
    {
      raw: vi.fn(),
    }
  );

  return {
    db,
    eq,
    sql,
    tables,
    reset() {
      selectResults.length = 0;
      executeResults.length = 0;
      db.select.mockClear();
      db.execute.mockClear();
      eq.mockClear();
      sql.raw.mockClear();
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

beforeAll(async () => {
  const module = await import("../../src/lib/embed/getUserEmbedStats");
  getUserEmbedStats = module.getUserEmbedStats;
});

beforeEach(() => {
  mockState.reset();
});

describe("getUserEmbedStats", () => {
  it("aggregates totals and submission count across multiple submission rows", async () => {
    mockState.pushSelectResult([
      {
        id: "user-1",
        username: "alice",
        displayName: "Alice",
        avatarUrl: null,
        totalTokens: 3100,
        totalCost: 17.75,
        submissionCount: 5,
        updatedAt: new Date("2026-04-01T08:00:00.000Z"),
      },
    ]);
    mockState.pushExecuteResult([{ rank: "2" }]);

    const result = await getUserEmbedStats("alice", "tokens");

    expect(result).toEqual({
      user: {
        id: "user-1",
        username: "alice",
        displayName: "Alice",
        avatarUrl: null,
      },
      stats: {
        totalTokens: 3100,
        totalCost: 17.75,
        submissionCount: 5,
        rank: 2,
        updatedAt: "2026-04-01T08:00:00.000Z",
      },
    });
  });

  it("returns null when the user is not found", async () => {
    mockState.pushSelectResult([]);

    const result = await getUserEmbedStats("missing-user", "cost");

    expect(result).toBeNull();
    expect(mockState.db.execute).not.toHaveBeenCalled();
  });
});
