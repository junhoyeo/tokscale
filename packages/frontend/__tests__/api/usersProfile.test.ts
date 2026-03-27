import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

const mockState = vi.hoisted(() => {
  const selectResults: Array<Array<Record<string, unknown>>> = [];
  const executeResults: Array<Array<Record<string, unknown>>> = [];

  const tables = {
    users: {
      id: "users.id",
      username: "users.username",
      displayName: "users.displayName",
      avatarUrl: "users.avatarUrl",
      createdAt: "users.createdAt",
    },
    submissions: {
      userId: "submissions.userId",
      totalTokens: "submissions.totalTokens",
      totalCost: "submissions.totalCost",
      inputTokens: "submissions.inputTokens",
      outputTokens: "submissions.outputTokens",
      cacheReadTokens: "submissions.cacheReadTokens",
      cacheCreationTokens: "submissions.cacheCreationTokens",
      reasoningTokens: "submissions.reasoningTokens",
      submitCount: "submissions.submitCount",
      dateStart: "submissions.dateStart",
      dateEnd: "submissions.dateEnd",
      sourcesUsed: "submissions.sourcesUsed",
      modelsUsed: "submissions.modelsUsed",
      updatedAt: "submissions.updatedAt",
      cliVersion: "submissions.cliVersion",
      schemaVersion: "submissions.schemaVersion",
    },
    dailyBreakdown: {
      submissionId: "dailyBreakdown.submissionId",
      date: "dailyBreakdown.date",
      timestampMs: "dailyBreakdown.timestampMs",
      tokens: "dailyBreakdown.tokens",
      cost: "dailyBreakdown.cost",
      inputTokens: "dailyBreakdown.inputTokens",
      outputTokens: "dailyBreakdown.outputTokens",
      sourceBreakdown: "dailyBreakdown.sourceBreakdown",
      modelBreakdown: "dailyBreakdown.modelBreakdown",
    },
  };

  const eq = vi.fn(() => "eq");
  const desc = vi.fn(() => "desc");
  const and = vi.fn(() => "and");
  const gte = vi.fn(() => "gte");
  const sql = Object.assign(
    () => ({
      as: () => ({}),
    }),
    {
      raw: vi.fn(),
    }
  );

  function nextSelectResult() {
    return selectResults.shift() ?? [];
  }

  const db = {
    select: vi.fn(() => {
      const builder = {
        from: vi.fn(() => builder),
        where: vi.fn(() => builder),
        innerJoin: vi.fn(() => builder),
        orderBy: vi.fn(() => builder),
        limit: vi.fn(() => builder),
        then: (resolve: (value: unknown) => unknown) => resolve(nextSelectResult()),
      };

      return builder;
    }),
    execute: vi.fn(async () => executeResults.shift() ?? []),
  };

  return {
    db,
    tables,
    eq,
    desc,
    and,
    gte,
    sql,
    reset() {
      selectResults.length = 0;
      executeResults.length = 0;
      db.select.mockClear();
      db.execute.mockClear();
      eq.mockClear();
      desc.mockClear();
      and.mockClear();
      gte.mockClear();
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
  sql: mockState.sql,
  and: mockState.and,
  gte: mockState.gte,
}));

type ModuleExports = typeof import("../../src/app/api/users/[username]/route");

let GET: ModuleExports["GET"];

beforeAll(async () => {
  const routeModule = await import("../../src/app/api/users/[username]/route");
  GET = routeModule.GET;
});

beforeEach(() => {
  mockState.reset();
});

afterEach(() => {
  vi.useRealTimers();
});

describe("GET /api/users/[username]", () => {
  it("returns submission freshness metadata for the latest submission", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-03-11T12:00:00.000Z"));

    mockState.pushSelectResult([
      {
        id: "user-1",
        username: "alice",
        displayName: "Alice",
        avatarUrl: null,
        createdAt: "2026-01-01T00:00:00.000Z",
      },
    ]);
    mockState.pushSelectResult([
      {
        totalTokens: 1200,
        totalCost: 12.5,
        inputTokens: 700,
        outputTokens: 500,
        cacheReadTokens: 100,
        cacheCreationTokens: 50,
        reasoningTokens: 25,
        submissionCount: 2,
        earliestDate: "2026-01-01",
        latestDate: "2026-03-10",
      },
    ]);
    mockState.pushSelectResult([
      {
        sourcesUsed: ["cursor"],
        modelsUsed: ["claude-3-7-sonnet"],
        updatedAt: new Date("2026-01-10T10:00:00.000Z"),
        cliVersion: "1.4.2",
        schemaVersion: 1,
      },
    ]);
    mockState.pushSelectResult([]);
    mockState.pushExecuteResult([{ rank: 3 }]);

    const response = await GET(
      new Request("http://localhost:3000/api/users/alice"),
      { params: Promise.resolve({ username: "alice" }) }
    );
    const body = await response.json();

    expect(response.status).toBe(200);
    expect(body.submissionFreshness).toEqual({
      lastUpdated: "2026-01-10T10:00:00.000Z",
      cliVersion: "1.4.2",
      schemaVersion: 1,
      isStale: true,
    });
    expect(body.updatedAt).toBe("2026-01-10T10:00:00.000Z");
    expect(body.clients).toEqual(["cursor"]);
    expect(body.models).toEqual(["claude-3-7-sonnet"]);
  });

  it("returns null freshness metadata when the user has no submission yet", async () => {
    mockState.pushSelectResult([
      {
        id: "user-2",
        username: "new-user",
        displayName: null,
        avatarUrl: null,
        createdAt: "2026-03-01T00:00:00.000Z",
      },
    ]);
    mockState.pushSelectResult([
      {
        totalTokens: 0,
        totalCost: 0,
        inputTokens: 0,
        outputTokens: 0,
        cacheReadTokens: 0,
        cacheCreationTokens: 0,
        reasoningTokens: 0,
        submissionCount: 0,
        earliestDate: null,
        latestDate: null,
      },
    ]);
    mockState.pushSelectResult([]);
    mockState.pushSelectResult([]);
    mockState.pushExecuteResult([]);

    const response = await GET(
      new Request("http://localhost:3000/api/users/new-user"),
      { params: Promise.resolve({ username: "new-user" }) }
    );
    const body = await response.json();

    expect(response.status).toBe(200);
    expect(body.submissionFreshness).toBeNull();
    expect(body.updatedAt).toBeNull();
    expect(body.clients).toEqual([]);
    expect(body.models).toEqual([]);
  });
});
