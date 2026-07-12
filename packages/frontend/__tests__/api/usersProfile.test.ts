import {
  afterEach,
  beforeAll,
  beforeEach,
  describe,
  expect,
  it,
  vi,
} from "vitest";

import { expectNoNarrowedCostCast } from "../support/costCastWidths";

const mockState = vi.hoisted(() => {
  const selectResults: Array<Array<Record<string, unknown>>> = [];
  const executeResults: Array<Array<Record<string, unknown>>> = [];
  const limitCalls: unknown[] = [];

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
      activeTimeMs: "dailyBreakdown.activeTimeMs",
      tokens: "dailyBreakdown.tokens",
      cost: "dailyBreakdown.cost",
      inputTokens: "dailyBreakdown.inputTokens",
      outputTokens: "dailyBreakdown.outputTokens",
      sourceBreakdown: "dailyBreakdown.sourceBreakdown",
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
    },
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
        limit: vi.fn((value: unknown) => {
          limitCalls.push(value);
          return builder;
        }),
        then: (resolve: (value: unknown) => unknown) =>
          resolve(nextSelectResult()),
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
    lte,
    sql,
    reset() {
      selectResults.length = 0;
      executeResults.length = 0;
      limitCalls.length = 0;
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
    pushSelectResult(rows: Array<Record<string, unknown>>) {
      selectResults.push(rows);
    },
    pushExecuteResult(rows: Array<Record<string, unknown>>) {
      executeResults.push(rows);
    },
    limitCalls,
  };
});

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
        throw new AmbiguousUsernameError(
          `Multiple users match username ${username} case-insensitively`,
        );
      }
      return rows[0] ?? null;
    },
    normalizeUsernameCacheKey: (username: string) => username.toLowerCase(),
    usernameEqualsIgnoreCase: (username: string) =>
      mockState.sql`lower(${mockState.tables.users.username}) = ${username.toLowerCase()}`,
  };
});

vi.mock(
  "@/lib/submissionFreshness",
  async () => import("../../src/lib/submissionFreshness"),
);

vi.mock("drizzle-orm", () => ({
  eq: mockState.eq,
  desc: mockState.desc,
  sql: mockState.sql,
  and: mockState.and,
  gte: mockState.gte,
  lte: mockState.lte,
}));

type ModuleExports = typeof import("../../src/app/api/users/[username]/route");

let GET: ModuleExports["GET"];

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
  it("redirects mixed-case requests to the canonical username path", async () => {
    mockState.pushSelectResult([
      {
        id: "user-imlunahey",
        username: "ImLunaHey",
        displayName: "Luna",
        avatarUrl: null,
        createdAt: "2026-01-01T00:00:00.000Z",
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
      new Request("http://localhost:3000/api/users/imlunahey"),
      { params: Promise.resolve({ username: "imlunahey" }) },
    );
    const sqlTexts = serializeSqlCalls();

    expect(response.status).toBe(308);
    expect(response.headers.get("location")).toBe(
      "http://localhost:3000/api/users/ImLunaHey",
    );
    expect(mockState.limitCalls[0]).toBe(2);
    expect(
      sqlTexts.some((text) =>
        text.toLowerCase().includes("lower(users.username) = imlunahey"),
      ),
    ).toBe(true);
  });

  it("returns the profile payload when the request already uses the canonical username", async () => {
    mockState.pushSelectResult([
      {
        id: "user-imlunahey",
        username: "ImLunaHey",
        displayName: "Luna",
        avatarUrl: null,
        createdAt: "2026-01-01T00:00:00.000Z",
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
      new Request("http://localhost:3000/api/users/ImLunaHey"),
      { params: Promise.resolve({ username: "ImLunaHey" }) },
    );
    const body = await response.json();

    expect(response.status).toBe(200);
    expect(body.user.username).toBe("ImLunaHey");
  });

  it("casts total_cost at full column precision in the profile stats query", async () => {
    mockState.pushSelectResult([
      {
        id: "user-alice",
        username: "alice",
        displayName: "Alice",
        avatarUrl: null,
        createdAt: "2026-01-01T00:00:00.000Z",
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

    await GET(new Request("http://localhost:3000/api/users/alice"), {
      params: Promise.resolve({ username: "alice" }),
    });

    // submissions.total_cost is decimal(18,4); a narrower cast overflows for a
    // profile whose lifetime cost has grown past the narrowed ceiling.
    expectNoNarrowedCostCast(serializeSqlCalls());
  });

  it("rejects ambiguous case-insensitive username matches", async () => {
    mockState.pushSelectResult([
      {
        id: "user-1",
        username: "ImLunaHey",
        displayName: "Luna",
        avatarUrl: null,
        createdAt: "2026-01-01T00:00:00.000Z",
      },
      {
        id: "user-2",
        username: "imlunahey",
        displayName: "Luna Duplicate",
        avatarUrl: null,
        createdAt: "2026-01-02T00:00:00.000Z",
      },
    ]);

    const response = await GET(
      new Request("http://localhost:3000/api/users/imlunahey"),
      { params: Promise.resolve({ username: "imlunahey" }) },
    );
    const body = await response.json();

    expect(response.status).toBe(409);
    expect(body).toEqual({ error: "Username is ambiguous" });
    expect(mockState.limitCalls[0]).toBe(2);
  });

  it("aggregates same-date rows from multiple submitted devices into one profile contribution", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-07-12T12:00:00.000Z"));

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
        totalTokens: 37,
        totalCost: 2.25,
        inputTokens: 27,
        outputTokens: 10,
        cacheReadTokens: 0,
        cacheCreationTokens: 0,
        reasoningTokens: 0,
        submissionCount: 2,
        earliestDate: "2026-04-30",
        latestDate: "2026-04-30",
        totalActiveTimeMs: 0,
        sessionCount: 0,
      },
    ]);
    mockState.pushSelectResult([
      {
        sourcesUsed: ["codex"],
        modelsUsed: ["gpt-5.5"],
        updatedAt: new Date("2026-04-30T12:00:00.000Z"),
        cliVersion: "2.0.0",
        schemaVersion: 2,
      },
    ]);
    mockState.pushSelectResult([
      {
        date: "2024-06-06",
        timestampMs: 50,
        tokens: 10,
        cost: "1.0000",
        inputTokens: 10,
        outputTokens: 0,
        sourceBreakdown: {
          codex: {
            tokens: 10,
            cost: 1,
            input: 10,
            output: 0,
            cacheRead: 0,
            cacheWrite: 0,
            reasoning: 0,
            messages: 1,
            models: {
              "gpt-legacy": {
                tokens: 10,
                cost: 1,
                input: 10,
                output: 0,
                cacheRead: 0,
                cacheWrite: 0,
                reasoning: 0,
                messages: 1,
              },
            },
          },
        },
      },
      {
        date: "2026-04-30",
        timestampMs: 100,
        tokens: 12,
        cost: "0.5000",
        inputTokens: 7,
        outputTokens: 5,
      sourceBreakdown: {
        codex: {
          tokens: 12,
            cost: 0.5,
            input: 7,
            output: 5,
            cacheRead: 0,
          cacheWrite: 0,
          reasoning: 0,
          messages: 1,
          modelId: "gpt-legacy",
          },
        },
      },
      {
        date: "2026-04-30",
        timestampMs: 200,
        tokens: 15,
        cost: "0.7500",
        inputTokens: 10,
        outputTokens: 5,
        sourceBreakdown: {
          codex: {
            tokens: 15,
            cost: 0.75,
            input: 10,
            output: 5,
            cacheRead: 0,
            cacheWrite: 0,
            reasoning: 0,
            messages: 1,
            models: {
              "gpt-5.5": {
                tokens: 15,
                cost: 0.75,
                input: 10,
                output: 5,
                cacheRead: 0,
                cacheWrite: 0,
                reasoning: 0,
                messages: 1,
              },
            },
          },
        },
      },
    ]);
    mockState.pushExecuteResult([{ rank: 4 }]);

    const response = await GET(
      new Request("http://localhost:3000/api/users/alice"),
      { params: Promise.resolve({ username: "alice" }) },
    );
    const body = await response.json();

    expect(response.status).toBe(200);
    expect(mockState.gte).not.toHaveBeenCalled();
    expect(body.stats.totalTokens).toBe(37);
    expect(body.chartRange).toEqual({
      start: "2025-07-12",
      end: "2026-07-12",
    });
    expect(body.stats.activeDays).toBe(1);
    expect(body.contributions).toHaveLength(2);
    expect(body.contributions.map((day: { date: string }) => day.date)).toEqual(
      ["2024-06-06", "2026-04-30"],
    );
    expect(body.contributions[1]).toEqual(
      expect.objectContaining({
        date: "2026-04-30",
        timestampMs: 100,
        totals: expect.objectContaining({
          tokens: 27,
          cost: 1.25,
        }),
        tokenBreakdown: expect.objectContaining({
          input: 17,
          output: 10,
        }),
      }),
    );
    expect(body.contributions[1].clients[0]).toEqual(
      expect.objectContaining({
        client: "codex",
        cost: 1.25,
        messages: 2,
        tokens: expect.objectContaining({
          input: 17,
          output: 10,
        }),
      }),
    );
    expect(body.contributions[1].clients[0].models).toEqual(
      expect.objectContaining({
        "gpt-5.5": expect.objectContaining({ tokens: 15, cost: 0.75 }),
        "gpt-legacy": expect.objectContaining({ tokens: 12, cost: 0.5 }),
      }),
    );
    expect(body.modelUsage).toEqual([
      expect.objectContaining({
        model: "gpt-5.5",
        tokens: 15,
        cost: 0.75,
        percentage: 60,
      }),
      expect.objectContaining({
        model: "gpt-legacy",
        tokens: 12,
        cost: 0.5,
        percentage: 40,
      }),
    ]);
  });

  it("clamps leap-day rolling ranges to the prior year's last valid day", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2024-02-29T12:00:00.000Z"));

    mockState.pushSelectResult([
      {
        id: "user-leap-day",
        username: "alice",
        displayName: "Alice",
        avatarUrl: null,
        createdAt: "2024-01-01T00:00:00.000Z",
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
      new Request("http://localhost:3000/api/users/alice"),
      { params: Promise.resolve({ username: "alice" }) },
    );
    const body = await response.json();

    expect(response.status).toBe(200);
    expect(body.chartRange).toEqual({
      start: "2023-02-28",
      end: "2024-02-29",
    });
  });

  it("recalculates profile overview stats from daily rows for rolling periods", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-06-28T12:00:00.000Z"));

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
        totalTokens: 1000,
        totalCost: 10,
        inputTokens: 600,
        outputTokens: 400,
        cacheReadTokens: 100,
        cacheCreationTokens: 50,
        reasoningTokens: 25,
        submissionCount: 3,
        earliestDate: "2026-01-01",
        latestDate: "2026-06-28",
        totalActiveTimeMs: 1200000,
        sessionCount: 8,
      },
    ]);
    mockState.pushSelectResult([
      {
        sourcesUsed: ["codex", "claude"],
        modelsUsed: ["gpt-5.5", "claude-sonnet-4-5"],
        updatedAt: new Date("2026-06-28T10:00:00.000Z"),
        cliVersion: "2.0.0",
        schemaVersion: 2,
      },
    ]);
    mockState.pushSelectResult([
      {
        date: "2026-06-22",
        timestampMs: 100,
        activeTimeMs: 300000,
        tokens: 200,
        cost: "2.0000",
        inputTokens: 120,
        outputTokens: 80,
        sourceBreakdown: {
          codex: {
            tokens: 200,
            cost: 2,
            input: 120,
            output: 80,
            cacheRead: 30,
            cacheWrite: 10,
            reasoning: 5,
            messages: 2,
            models: {
              "gpt-5.5": {
                tokens: 200,
                cost: 2,
                input: 120,
                output: 80,
                cacheRead: 30,
                cacheWrite: 10,
                reasoning: 5,
                messages: 2,
              },
            },
          },
        },
      },
      {
        date: "2026-06-28",
        timestampMs: 200,
        activeTimeMs: 600000,
        tokens: 300,
        cost: "3.0000",
        inputTokens: 180,
        outputTokens: 120,
        sourceBreakdown: {
          claude: {
            tokens: 300,
            cost: 3,
            input: 180,
            output: 120,
            cacheRead: 40,
            cacheWrite: 20,
            reasoning: 10,
            messages: 3,
            models: {
              "claude-sonnet-4-5": {
                tokens: 300,
                cost: 3,
                input: 180,
                output: 120,
                cacheRead: 40,
                cacheWrite: 20,
                reasoning: 10,
                messages: 3,
              },
            },
          },
        },
      },
    ]);
    mockState.pushExecuteResult([{ rank: 4 }]);

    const response = await GET(
      new Request("http://localhost:3000/api/users/alice?period=week"),
      { params: Promise.resolve({ username: "alice" }) },
    );
    const body = await response.json();

    expect(response.status).toBe(200);
    expect(mockState.gte).toHaveBeenCalledWith(
      mockState.tables.dailyBreakdown.date,
      "2026-06-22",
    );
    expect(mockState.lte).toHaveBeenCalledWith(
      mockState.tables.dailyBreakdown.date,
      "2026-06-28",
    );
    expect(body.period).toBe("week");
    expect(body.dateRange).toEqual({ start: "2026-06-22", end: "2026-06-28" });
    expect(body.stats).toEqual(
      expect.objectContaining({
        totalTokens: 500,
        totalCost: 5,
        inputTokens: 300,
        outputTokens: 200,
        cacheReadTokens: 70,
        cacheWriteTokens: 30,
        reasoningTokens: 15,
        activeDays: 2,
        totalActiveTimeMs: 900000,
        sessionCount: 0,
      }),
    );
    expect(body.clients).toEqual(["codex", "claude"]);
    expect(body.models).toEqual(["gpt-5.5", "claude-sonnet-4-5"]);
  });

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
      { params: Promise.resolve({ username: "alice" }) },
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
      { params: Promise.resolve({ username: "new-user" }) },
    );
    const body = await response.json();

    expect(response.status).toBe(200);
    expect(body.submissionFreshness).toBeNull();
    expect(body.updatedAt).toBeNull();
    expect(body.clients).toEqual([]);
    expect(body.models).toEqual([]);
  });
});
