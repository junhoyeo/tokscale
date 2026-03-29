import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

const mockState = vi.hoisted(() => {
  type Row = Record<string, unknown>;

  const selectResults: Row[][] = [];
  const insertReturningResults: Row[][] = [];
  const whereCalls: Array<{ table: unknown; condition: unknown }> = [];
  const insertCalls: Array<{ table: unknown; values: unknown }> = [];
  const updateCalls: Array<{ table: unknown; values: unknown; condition: unknown }> = [];

  const tables = {
    apiTokens: {
      id: "apiTokens.id",
    },
    submissions: {
      id: "submissions.id",
      userId: "submissions.userId",
      sourceId: "submissions.sourceId",
      sourceName: "submissions.sourceName",
      totalTokens: "submissions.totalTokens",
      totalCost: "submissions.totalCost",
      inputTokens: "submissions.inputTokens",
      outputTokens: "submissions.outputTokens",
      cacheCreationTokens: "submissions.cacheCreationTokens",
      cacheReadTokens: "submissions.cacheReadTokens",
      reasoningTokens: "submissions.reasoningTokens",
      dateStart: "submissions.dateStart",
      dateEnd: "submissions.dateEnd",
      sourcesUsed: "submissions.sourcesUsed",
      modelsUsed: "submissions.modelsUsed",
      submitCount: "submissions.submitCount",
      schemaVersion: "submissions.schemaVersion",
      updatedAt: "submissions.updatedAt",
    },
    dailyBreakdown: {
      id: "dailyBreakdown.id",
      submissionId: "dailyBreakdown.submissionId",
      date: "dailyBreakdown.date",
      tokens: "dailyBreakdown.tokens",
      cost: "dailyBreakdown.cost",
      inputTokens: "dailyBreakdown.inputTokens",
      outputTokens: "dailyBreakdown.outputTokens",
      timestampMs: "dailyBreakdown.timestampMs",
      sourceBreakdown: "dailyBreakdown.sourceBreakdown",
      modelBreakdown: "dailyBreakdown.modelBreakdown",
    },
  };

  const eq = vi.fn((left: unknown, right: unknown) => ({ type: "eq", left, right }));
  const and = vi.fn((...conditions: unknown[]) => ({ type: "and", conditions }));
  const isNull = vi.fn((value: unknown) => ({ type: "isNull", value }));
  const sql = Object.assign(
    vi.fn(() => ({ kind: "sql" })),
    {
      join: vi.fn(() => ({ kind: "sql.join" })),
    }
  );

  function nextSelectResult() {
    return selectResults.shift() ?? [];
  }

  function nextInsertReturningResult() {
    return insertReturningResults.shift() ?? [];
  }

  function createSelectBuilder() {
    let table: unknown;
    const builder = {
      from: vi.fn((nextTable: unknown) => {
        table = nextTable;
        return builder;
      }),
      innerJoin: vi.fn(() => builder),
      where: vi.fn((condition: unknown) => {
        whereCalls.push({ table, condition });
        return builder;
      }),
      for: vi.fn(() => builder),
      limit: vi.fn(() => builder),
      then: (resolve: (value: unknown) => unknown) => resolve(nextSelectResult()),
    };
    return builder;
  }

  function createInsertBuilder(table: unknown) {
    return {
      values: vi.fn((values: unknown) => {
        insertCalls.push({ table, values });
        return {
          returning: vi.fn(async () => nextInsertReturningResult()),
          then: (resolve: (value: unknown) => unknown) => resolve([]),
        };
      }),
    };
  }

  function createUpdateBuilder(table: unknown) {
    return {
      set: vi.fn((values: unknown) => ({
        where: vi.fn(async (condition: unknown) => {
          updateCalls.push({ table, values, condition });
          return [];
        }),
      })),
    };
  }

  const transaction = vi.fn(async (callback: (tx: unknown) => Promise<unknown>) => {
    const tx = {
      select: vi.fn(() => createSelectBuilder()),
      insert: vi.fn((table: unknown) => createInsertBuilder(table)),
      update: vi.fn((table: unknown) => createUpdateBuilder(table)),
      execute: vi.fn(async () => []),
    };
    return callback(tx);
  });

  return {
    authenticatePersonalToken: vi.fn(),
    validateSubmission: vi.fn(),
    generateSubmissionHash: vi.fn(() => "submission-hash"),
    revalidateTag: vi.fn(),
    db: {
      transaction,
    },
    tables,
    eq,
    and,
    isNull,
    sql,
    whereCalls,
    insertCalls,
    updateCalls,
    reset() {
      selectResults.length = 0;
      insertReturningResults.length = 0;
      whereCalls.length = 0;
      insertCalls.length = 0;
      updateCalls.length = 0;
      this.authenticatePersonalToken.mockReset();
      this.validateSubmission.mockReset();
      this.generateSubmissionHash.mockClear();
      this.revalidateTag.mockClear();
      transaction.mockClear();
      eq.mockClear();
      and.mockClear();
      isNull.mockClear();
      sql.mockClear();
      sql.join.mockClear();
    },
    pushSelectResult(rows: Row[]) {
      selectResults.push(rows);
    },
    pushInsertReturningResult(rows: Row[]) {
      insertReturningResults.push(rows);
    },
  };
});

vi.mock("next/cache", () => ({
  revalidateTag: mockState.revalidateTag,
}));

vi.mock("@/lib/auth/personalTokens", () => ({
  authenticatePersonalToken: mockState.authenticatePersonalToken,
}));

vi.mock("@/lib/db", () => ({
  db: mockState.db,
  apiTokens: mockState.tables.apiTokens,
  submissions: mockState.tables.submissions,
  dailyBreakdown: mockState.tables.dailyBreakdown,
}));

vi.mock("@/lib/validation/submission", () => ({
  validateSubmission: mockState.validateSubmission,
  generateSubmissionHash: mockState.generateSubmissionHash,
}));

vi.mock("@/lib/db/helpers", () => ({
  mergeClientBreakdowns: vi.fn((_existing, incoming) => incoming),
  recalculateDayTotals: vi.fn((clientBreakdown) => {
    let tokens = 0;
    let cost = 0;
    let inputTokens = 0;
    let outputTokens = 0;
    for (const client of Object.values(clientBreakdown as Record<string, {
      tokens: number;
      cost: number;
      input: number;
      output: number;
    }>)) {
      tokens += client.tokens;
      cost += client.cost;
      inputTokens += client.input;
      outputTokens += client.output;
    }
    return {
      tokens,
      cost,
      inputTokens,
      outputTokens,
      cacheReadTokens: 0,
      cacheWriteTokens: 0,
      reasoningTokens: 0,
    };
  }),
  buildModelBreakdown: vi.fn(() => ({})),
  clientContributionToBreakdownData: vi.fn((client) => ({
    tokens: client.tokens.input + client.tokens.output + client.tokens.cacheRead + client.tokens.cacheWrite + (client.tokens.reasoning || 0),
    cost: client.cost,
    input: client.tokens.input,
    output: client.tokens.output,
    cacheRead: client.tokens.cacheRead,
    cacheWrite: client.tokens.cacheWrite,
    reasoning: client.tokens.reasoning || 0,
    messages: client.messages,
  })),
  mergeTimestampMs: vi.fn((existing, incoming) => incoming ?? existing ?? null),
}));

vi.mock("drizzle-orm", () => ({
  eq: mockState.eq,
  and: mockState.and,
  isNull: mockState.isNull,
  sql: mockState.sql,
}));

type ModuleExports = typeof import("../../src/app/api/submit/route");

let POST: ModuleExports["POST"];

function createSubmissionData(metaOverrides: Partial<{ sourceId: string; sourceName: string }> = {}) {
  return {
    meta: {
      generatedAt: "2026-03-29T00:00:00.000Z",
      version: "2.0.14",
      dateRange: {
        start: "2026-03-01",
        end: "2026-03-01",
      },
      ...metaOverrides,
    },
    summary: {
      totalTokens: 150,
      totalCost: 1.5,
      totalDays: 1,
      activeDays: 1,
      averagePerDay: 1.5,
      maxCostInSingleDay: 1.5,
      clients: ["claude"],
      models: ["claude-sonnet-4-20250514"],
    },
    years: [],
    contributions: [
      {
        date: "2026-03-01",
        totals: {
          tokens: 150,
          cost: 1.5,
          messages: 2,
        },
        intensity: 1,
        tokenBreakdown: {
          input: 100,
          output: 50,
          cacheRead: 0,
          cacheWrite: 0,
          reasoning: 0,
        },
        clients: [
          {
            client: "claude",
            modelId: "claude-sonnet-4-20250514",
            tokens: {
              input: 100,
              output: 50,
              cacheRead: 0,
              cacheWrite: 0,
              reasoning: 0,
            },
            cost: 1.5,
            messages: 2,
          },
        ],
      },
    ],
  };
}

beforeAll(async () => {
  const routeModule = await import("../../src/app/api/submit/route");
  POST = routeModule.POST;
});

beforeEach(() => {
  mockState.reset();
  mockState.authenticatePersonalToken.mockResolvedValue({
    status: "valid",
    tokenId: "token-1",
    userId: "user-1",
    username: "alice",
  });
});

function queueSuccessfulTransaction(existingSubmissionRows: Array<Record<string, unknown>> = []) {
  mockState.pushSelectResult(existingSubmissionRows);
  mockState.pushInsertReturningResult([{ id: "submission-1" }]);
  mockState.pushSelectResult([]);
  mockState.pushSelectResult([
    {
      totalTokens: 150,
      totalCost: "1.5000",
      inputTokens: 100,
      outputTokens: 50,
      dateStart: "2026-03-01",
      dateEnd: "2026-03-01",
      activeDays: 1,
      rowCount: 1,
    },
  ]);
  mockState.pushSelectResult([
    {
      sourceBreakdown: {
        claude: {
          tokens: 150,
          cost: 1.5,
          input: 100,
          output: 50,
          cacheRead: 0,
          cacheWrite: 0,
          reasoning: 0,
          messages: 2,
          models: {
            "claude-sonnet-4-20250514": {
              tokens: 150,
              cost: 1.5,
              input: 100,
              output: 50,
              cacheRead: 0,
              cacheWrite: 0,
              reasoning: 0,
              messages: 2,
            },
          },
        },
      },
    },
  ]);
  mockState.pushSelectResult([
    {
      totalTokens: 150,
      totalCost: "1.5000",
      dateStart: "2026-03-01",
      dateEnd: "2026-03-01",
    },
  ]);
  mockState.pushSelectResult([{ activeDays: 1 }]);
  mockState.pushSelectResult([{ sourcesUsed: ["claude"] }]);
}

describe("POST /api/submit source scoping", () => {
  it("looks up and creates a source-scoped submission when sourceId is present", async () => {
    mockState.validateSubmission.mockReturnValue({
      valid: true,
      data: createSubmissionData({
        sourceId: "machine-a",
        sourceName: "MacBook Air",
      }),
      errors: [],
      warnings: [],
    });
    queueSuccessfulTransaction();

    const response = await POST(
      new Request("http://localhost:3000/api/submit", {
        method: "POST",
        headers: {
          Authorization: "Bearer tt_valid",
          "Content-Type": "application/json",
        },
        body: JSON.stringify({}),
      })
    );

    expect(response.status).toBe(200);
    expect(mockState.whereCalls[0]).toEqual({
      table: mockState.tables.submissions,
      condition: {
        type: "and",
        conditions: [
          { type: "eq", left: "submissions.userId", right: "user-1" },
          { type: "eq", left: "submissions.sourceId", right: "machine-a" },
        ],
      },
    });
    expect(mockState.insertCalls[0]).toEqual({
      table: mockState.tables.submissions,
      values: expect.objectContaining({
        userId: "user-1",
        sourceId: "machine-a",
        sourceName: "MacBook Air",
      }),
    });
  });

  it("uses the unsourced submission row when sourceId is absent", async () => {
    mockState.validateSubmission.mockReturnValue({
      valid: true,
      data: createSubmissionData(),
      errors: [],
      warnings: [],
    });
    queueSuccessfulTransaction();

    const response = await POST(
      new Request("http://localhost:3000/api/submit", {
        method: "POST",
        headers: {
          Authorization: "Bearer tt_valid",
          "Content-Type": "application/json",
        },
        body: JSON.stringify({}),
      })
    );

    expect(response.status).toBe(200);
    expect(mockState.whereCalls[0]).toEqual({
      table: mockState.tables.submissions,
      condition: {
        type: "and",
        conditions: [
          { type: "eq", left: "submissions.userId", right: "user-1" },
          { type: "isNull", value: "submissions.sourceId" },
        ],
      },
    });
    expect(mockState.insertCalls[0]).toEqual({
      table: mockState.tables.submissions,
      values: expect.objectContaining({
        userId: "user-1",
        sourceId: null,
        sourceName: null,
      }),
    });
  });
});
