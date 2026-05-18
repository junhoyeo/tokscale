import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

const mockState = vi.hoisted(() => {
  const authenticatePersonalToken = vi.fn();
  const validateSubmission = vi.fn();
  const revalidateTag = vi.fn();
  const revalidatePath = vi.fn();
  const mergeClientBreakdowns = vi.fn();
  const recalculateDayTotals = vi.fn();
  const buildModelBreakdown = vi.fn();
  const clientContributionToBreakdownData = vi.fn();
  const mergeTimestampMs = vi.fn();
  const resolveSubmissionScope = vi.fn();
  const selectResults: Array<Array<Record<string, unknown>>> = [];

  const submissions = {
    id: "submissions.id",
    userId: "submissions.userId",
    sourceId: "submissions.sourceId",
    totalTokens: "submissions.totalTokens",
    totalCost: "submissions.totalCost",
    inputTokens: "submissions.inputTokens",
    outputTokens: "submissions.outputTokens",
    cacheCreationTokens: "submissions.cacheCreationTokens",
    cacheReadTokens: "submissions.cacheReadTokens",
    schemaVersion: "submissions.schemaVersion",
    updatedAt: "submissions.updatedAt",
    cliVersion: "submissions.cliVersion",
    dateStart: "submissions.dateStart",
    dateEnd: "submissions.dateEnd",
    sourcesUsed: "submissions.sourcesUsed",
    modelsUsed: "submissions.modelsUsed",
  };

  const dailyBreakdown = {
    id: "dailyBreakdown.id",
    submissionId: "dailyBreakdown.submissionId",
    date: "dailyBreakdown.date",
    tokens: "dailyBreakdown.tokens",
    cost: "dailyBreakdown.cost",
    inputTokens: "dailyBreakdown.inputTokens",
    outputTokens: "dailyBreakdown.outputTokens",
    timestampMs: "dailyBreakdown.timestampMs",
    sourceBreakdown: "dailyBreakdown.sourceBreakdown",
  };

  const apiTokens = {
    id: "apiTokens.id",
    lastUsedAt: "apiTokens.lastUsedAt",
  };

  const eq = vi.fn(() => "eq");
  const and = vi.fn(() => "and");
  const isNull = vi.fn(() => "isNull");
  const sql = Object.assign(
    () => ({
      as: () => ({}),
    }),
    {
      raw: vi.fn(),
    }
  );

  const db = {
    transaction: vi.fn(),
    select: vi.fn(() => {
      const builder = {
        from: vi.fn(() => builder),
        where: vi.fn(() => builder),
        innerJoin: vi.fn(() => builder),
        then: (resolve: (value: unknown) => unknown) =>
          resolve(selectResults.shift() ?? []),
      };

      return builder;
    }),
  };

  return {
    authenticatePersonalToken,
    validateSubmission,
    revalidateTag,
    revalidatePath,
    mergeClientBreakdowns,
    recalculateDayTotals,
    buildModelBreakdown,
    clientContributionToBreakdownData,
    mergeTimestampMs,
    resolveSubmissionScope,
    apiTokens,
    submissions,
    dailyBreakdown,
    eq,
    and,
    isNull,
    sql,
    db,
    reset() {
      authenticatePersonalToken.mockReset();
      validateSubmission.mockReset();
      revalidateTag.mockClear();
      revalidatePath.mockClear();
      mergeClientBreakdowns.mockReset();
      recalculateDayTotals.mockReset();
      buildModelBreakdown.mockReset();
      clientContributionToBreakdownData.mockReset();
      mergeTimestampMs.mockReset();
      resolveSubmissionScope.mockReset();
      db.transaction.mockReset();
      db.select.mockClear();
      selectResults.length = 0;
      eq.mockClear();
      and.mockClear();
      isNull.mockClear();
      sql.raw.mockClear();
    },
    pushSelectResult(rows: Array<Record<string, unknown>>) {
      selectResults.push(rows);
    },
  };
});

vi.mock("next/cache", () => ({
  revalidateTag: mockState.revalidateTag,
  revalidatePath: mockState.revalidatePath,
}));

vi.mock("@/lib/auth/personalTokens", () => ({
  authenticatePersonalToken: mockState.authenticatePersonalToken,
}));

vi.mock("@/lib/db", () => ({
  db: mockState.db,
  apiTokens: mockState.apiTokens,
  submissions: mockState.submissions,
  dailyBreakdown: mockState.dailyBreakdown,
}));

vi.mock("@/lib/validation/submission", () => ({
  validateSubmission: mockState.validateSubmission,
}));

vi.mock("@/lib/db/helpers", () => ({
  mergeClientBreakdowns: mockState.mergeClientBreakdowns,
  recalculateDayTotals: mockState.recalculateDayTotals,
  buildModelBreakdown: mockState.buildModelBreakdown,
  clientContributionToBreakdownData: mockState.clientContributionToBreakdownData,
  mergeTimestampMs: mockState.mergeTimestampMs,
  resolveSubmissionScope: mockState.resolveSubmissionScope,
}));

vi.mock("drizzle-orm", () => ({
  eq: mockState.eq,
  and: mockState.and,
  isNull: mockState.isNull,
  sql: mockState.sql,
}));

type ModuleExports = typeof import("../../src/app/api/submit/route");

let POST: ModuleExports["POST"];
let SourceIdentityRequiredError: ModuleExports["SourceIdentityRequiredError"];

beforeAll(async () => {
  const routeModule = await import("../../src/app/api/submit/route");
  POST = routeModule.POST;
  SourceIdentityRequiredError = routeModule.SourceIdentityRequiredError;
});

beforeEach(() => {
  mockState.reset();
});

describe("POST /api/submit auth path", () => {
  it("rejects invalid API tokens through the shared auth service", async () => {
    mockState.authenticatePersonalToken.mockResolvedValue({ status: "invalid" });

    const response = await POST(
      new Request("http://localhost:3000/api/submit", {
        method: "POST",
        headers: {
          Authorization: "Bearer tt_invalid",
        },
        body: JSON.stringify({}),
      })
    );

    expect(response.status).toBe(401);
    expect(mockState.authenticatePersonalToken).toHaveBeenCalledWith("tt_invalid", {
      touchLastUsedAt: false,
    });
    expect(await response.json()).toEqual({ error: "Invalid API token" });
  });

  it("returns the expired-token error without entering the transaction path", async () => {
    mockState.authenticatePersonalToken.mockResolvedValue({ status: "expired" });

    const response = await POST(
      new Request("http://localhost:3000/api/submit", {
        method: "POST",
        headers: {
          Authorization: "Bearer tt_expired",
        },
        body: JSON.stringify({}),
      })
    );

    expect(response.status).toBe(401);
    expect(mockState.authenticatePersonalToken).toHaveBeenCalledWith("tt_expired", {
      touchLastUsedAt: false,
    });
    expect(await response.json()).toEqual({ error: "API token has expired" });
    expect(mockState.db.transaction).not.toHaveBeenCalled();
  });

  it("accepts a valid token and continues into submission validation", async () => {
    mockState.authenticatePersonalToken.mockResolvedValue({
      status: "valid",
      tokenId: "token-1",
      userId: "user-1",
      username: "alice",
      displayName: "Alice",
      avatarUrl: null,
      isAdmin: false,
      expiresAt: null,
    });
    mockState.validateSubmission.mockReturnValue({
      valid: false,
      data: null,
      errors: ["bad payload"],
    });

    const response = await POST(
      new Request("http://localhost:3000/api/submit", {
        method: "POST",
        headers: {
          Authorization: "Bearer tt_valid",
          "Content-Type": "application/json",
        },
        body: JSON.stringify({ meta: {}, contributions: [] }),
      })
    );

    expect(response.status).toBe(400);
    expect(mockState.authenticatePersonalToken).toHaveBeenCalledWith("tt_valid", {
      touchLastUsedAt: false,
    });
    expect(mockState.validateSubmission).toHaveBeenCalledTimes(1);
    expect(mockState.db.transaction).not.toHaveBeenCalled();
    expect(await response.json()).toEqual({
      error: "Validation failed",
      details: ["bad payload"],
    });
  });

  it("returns 409 when source identity is required after scoped mode begins", async () => {
    mockState.authenticatePersonalToken.mockResolvedValue({
      status: "valid",
      tokenId: "token-1",
      userId: "user-1",
      username: "alice",
      displayName: "Alice",
      avatarUrl: null,
      isAdmin: false,
      expiresAt: null,
    });
    mockState.validateSubmission.mockReturnValue({
      valid: true,
      data: {
        meta: {
          generatedAt: new Date().toISOString(),
          version: "1.0.0",
          dateRange: { start: "2024-12-01", end: "2024-12-01" },
        },
        summary: {
          totalTokens: 1500,
          totalCost: 1.5,
          totalDays: 1,
          activeDays: 1,
          averagePerDay: 1.5,
          maxCostInSingleDay: 1.5,
          clients: ["claude"],
          models: ["claude-sonnet-4"],
        },
        years: [],
        contributions: [
          {
            date: "2024-12-01",
            totals: { tokens: 1500, cost: 1.5, messages: 5 },
            intensity: 2,
            tokenBreakdown: {
              input: 1000,
              output: 500,
              cacheRead: 0,
              cacheWrite: 0,
              reasoning: 0,
            },
            clients: [
              {
                client: "claude",
                modelId: "claude-sonnet-4",
                tokens: {
                  input: 1000,
                  output: 500,
                  cacheRead: 0,
                  cacheWrite: 0,
                  reasoning: 0,
                },
                cost: 1.5,
                messages: 5,
              },
            ],
          },
        ],
      },
      errors: [],
      warnings: [],
    });
    mockState.db.transaction.mockRejectedValue(new SourceIdentityRequiredError());

    const response = await POST(
      new Request("http://localhost:3000/api/submit", {
        method: "POST",
        headers: {
          Authorization: "Bearer tt_valid",
          "Content-Type": "application/json",
        },
        body: JSON.stringify({ meta: {}, contributions: [] }),
      })
    );

    expect(response.status).toBe(409);
    expect(await response.json()).toEqual({
      error: "Source identity is required for accounts with source-scoped submissions",
      hint: "Upgrade the CLI or set TOKSCALE_SOURCE_ID before submitting from this machine.",
    });
  });

  it("end-to-end rollout flow: scoped account rejects an unsourced submit with 409+hint", async () => {
    // Rollout scenario: a user has already submitted from a new-CLI machine
    // with meta.sourceId, so their submissions table has source-scoped rows.
    // An older CLI on a different machine then submits WITHOUT sourceId.
    // We want the real route code (transaction callback → resolveSubmissionScope
    // → SourceIdentityRequiredError throw → outer catch) to end at 409+hint.
    mockState.authenticatePersonalToken.mockResolvedValue({
      status: "valid",
      tokenId: "token-1",
      userId: "user-1",
      username: "alice",
      displayName: "Alice",
      avatarUrl: null,
      isAdmin: false,
      expiresAt: null,
    });
    mockState.validateSubmission.mockReturnValue({
      valid: true,
      data: {
        // Note: NO meta.sourceId — simulates old CLI.
        meta: {
          generatedAt: new Date().toISOString(),
          version: "0.9.0",
          dateRange: { start: "2024-12-01", end: "2024-12-01" },
        },
        summary: {
          totalTokens: 100,
          totalCost: 0.1,
          totalDays: 1,
          activeDays: 1,
          averagePerDay: 0.1,
          maxCostInSingleDay: 0.1,
          clients: ["claude"],
          models: ["claude-sonnet-4"],
        },
        years: [],
        contributions: [
          {
            date: "2024-12-01",
            totals: { tokens: 100, cost: 0.1, messages: 1 },
            intensity: 1,
            tokenBreakdown: {
              input: 60,
              output: 40,
              cacheRead: 0,
              cacheWrite: 0,
              reasoning: 0,
            },
            clients: [
              {
                client: "claude",
                modelId: "claude-sonnet-4",
                tokens: { input: 60, output: 40, cacheRead: 0, cacheWrite: 0, reasoning: 0 },
                cost: 0.1,
                messages: 1,
              },
            ],
          },
        ],
      },
      errors: [],
      warnings: [],
    });

    // The account already has a source-scoped row; resolveSubmissionScope
    // sees sourceId=null and rows with non-null source_id → reject.
    mockState.resolveSubmissionScope.mockReturnValue({
      kind: "rejectMissingSourceIdentity",
    });

    // Drive the real route code through the transaction callback so
    // SourceIdentityRequiredError is thrown from within the route (not
    // mocked at the transaction boundary).
    mockState.db.transaction.mockImplementation(async (callback) => {
      const selectResults = [
        // 3a scope select: returns an existing source-scoped row.
        [{ id: "submission-scoped", sourceId: "machine-a" }],
      ];
      const tx = {
        update: vi.fn(() => ({
          set: vi.fn(() => ({
            where: vi.fn(async () => []),
          })),
        })),
        select: vi.fn(() => {
          const builder = {
            from: vi.fn(() => builder),
            innerJoin: vi.fn(() => builder),
            where: vi.fn(() => builder),
            limit: vi.fn(async () => selectResults.shift() ?? []),
            for: vi.fn(() => builder),
            then: (resolve: (value: unknown) => unknown) =>
              resolve(selectResults.shift() ?? []),
          };
          return builder;
        }),
        insert: vi.fn(() => ({ values: vi.fn(async () => []) })),
        execute: vi.fn(async () => []),
      };

      return callback(tx as never);
    });

    const response = await POST(
      new Request("http://localhost:3000/api/submit", {
        method: "POST",
        headers: {
          Authorization: "Bearer tt_valid",
          "Content-Type": "application/json",
        },
        body: JSON.stringify({ meta: {}, contributions: [] }),
      })
    );

    expect(response.status).toBe(409);
    expect(await response.json()).toEqual({
      error: "Source identity is required for accounts with source-scoped submissions",
      hint: "Upgrade the CLI or set TOKSCALE_SOURCE_ID before submitting from this machine.",
    });
    // resolveSubmissionScope saw the existing scoped row and a null sourceId.
    expect(mockState.resolveSubmissionScope).toHaveBeenCalledWith(
      [{ id: "submission-scoped", sourceId: "machine-a" }],
      null
    );
  });

  it('returns mode "merge" when the transaction falls back to an existing submission row', async () => {
    mockState.authenticatePersonalToken.mockResolvedValue({
      status: "valid",
      tokenId: "token-1",
      userId: "user-1",
      username: "alice",
      displayName: "Alice",
      avatarUrl: null,
      isAdmin: false,
      expiresAt: null,
    });
    mockState.validateSubmission.mockReturnValue({
      valid: true,
      data: {
        meta: {
          generatedAt: new Date().toISOString(),
          version: "1.0.0",
          dateRange: { start: "2024-12-01", end: "2024-12-01" },
        },
        summary: {
          totalTokens: 1500,
          totalCost: 1.5,
          totalDays: 1,
          activeDays: 1,
          averagePerDay: 1.5,
          maxCostInSingleDay: 1.5,
          clients: ["claude"],
          models: ["claude-sonnet-4"],
        },
        years: [],
        contributions: [
          {
            date: "2024-12-01",
            totals: { tokens: 1500, cost: 1.5, messages: 5 },
            intensity: 2,
            tokenBreakdown: {
              input: 1000,
              output: 500,
              cacheRead: 0,
              cacheWrite: 0,
              reasoning: 0,
            },
            clients: [
              {
                client: "claude",
                modelId: "claude-sonnet-4",
                tokens: {
                  input: 1000,
                  output: 500,
                  cacheRead: 0,
                  cacheWrite: 0,
                  reasoning: 0,
                },
                cost: 1.5,
                messages: 5,
              },
            ],
          },
        ],
      },
      errors: [],
      warnings: [],
    });
    const mockModelData = {
      tokens: 1500,
      cost: 1.5,
      input: 1000,
      output: 500,
      cacheRead: 0,
      cacheWrite: 0,
      reasoning: 0,
      messages: 5,
    };
    const mockSourceBreakdown = {
      claude: {
        ...mockModelData,
        models: {
          "claude-sonnet-4": { ...mockModelData },
        },
      },
    };

    mockState.clientContributionToBreakdownData.mockReturnValue(mockModelData);
    mockState.mergeClientBreakdowns.mockImplementation((_existing: unknown, incoming: unknown) => incoming);
    mockState.recalculateDayTotals.mockReturnValue({
      tokens: 1500,
      cost: 1.5,
      inputTokens: 1000,
      outputTokens: 500,
      cacheReadTokens: 0,
      cacheWriteTokens: 0,
      reasoningTokens: 0,
    });
    mockState.buildModelBreakdown.mockReturnValue({
      "claude-sonnet-4": 1500,
    });
    mockState.mergeTimestampMs.mockReturnValue(null);
    mockState.resolveSubmissionScope.mockReturnValue({ kind: "create" });

    mockState.db.transaction.mockImplementation(async (callback) => {
      const selectResults = [
        [], // 3a: scope select (.for → builder, awaited via .then)
        [{ id: "submission-1" }], // 3a fallback: .for(...).limit(1) → consumed by .limit
        [], // 3b: daily breakdown FOR UPDATE (.for → builder, awaited via .then)
        [
          {
            totalTokens: 1500,
            totalCost: "1.5000",
            inputTokens: 1000,
            outputTokens: 500,
            dateStart: "2024-12-01",
            dateEnd: "2024-12-01",
            activeDays: 1,
            rowCount: 1,
          },
        ], // 3d: aggregates
        [{ sourceBreakdown: mockSourceBreakdown }], // 3d: allDays
        [
          {
            totalTokens: 1500,
            totalCost: "1.5000",
            dateStart: "2024-12-01",
            dateEnd: "2024-12-01",
          },
        ], // metrics: user aggregates
        [{ activeDays: 1 }], // metrics: user day aggregates
        [{ sourcesUsed: ["claude"] }], // metrics: user submissions
      ];
      const tx = {
        update: vi.fn(() => {
          const builder = {
            set: vi.fn(() => ({
              where: vi.fn(async () => []),
            })),
          };
          return builder;
        }),
        select: vi.fn(() => {
          const builder = {
            from: vi.fn(() => builder),
            innerJoin: vi.fn(() => builder),
            where: vi.fn(() => builder),
            limit: vi.fn(async () => selectResults.shift() ?? []),
            // .for() is chainable — drizzle allows `.for(...).limit(1)` as
            // well as terminal `.for(...)`. Return the builder so chained
            // calls work; terminal `.for(...)` gets consumed via .then.
            for: vi.fn(() => builder),
            then: (resolve: (value: unknown) => unknown) =>
              resolve(selectResults.shift() ?? []),
          };
          return builder;
        }),
        insert: vi.fn((table: unknown) => {
          if (table === mockState.submissions) {
            return {
              values: vi.fn(() => ({
                onConflictDoNothing: vi.fn(() => ({
                  returning: vi.fn(async () => []),
                })),
              })),
            };
          }

          return {
            values: vi.fn(async () => []),
          };
        }),
        execute: vi.fn(async () => []),
      };

      return callback(tx as never);
    });

    const response = await POST(
      new Request("http://localhost:3000/api/submit", {
        method: "POST",
        headers: {
          Authorization: "Bearer tt_valid",
          "Content-Type": "application/json",
        },
        body: JSON.stringify({ meta: {}, contributions: [] }),
      })
    );
    const body = await response.json();

    expect(response.status).toBe(200);
    expect(body.mode).toBe("merge");
    expect(body.metrics).toEqual({
      totalTokens: 1500,
      totalCost: 1.5,
      dateRange: {
        start: "2024-12-01",
        end: "2024-12-01",
      },
      activeDays: 1,
      clients: ["claude"],
    });
  });

  it("preserves a user-renamed sourceName — CLI default does NOT overwrite on subsequent merges", async () => {
    mockState.authenticatePersonalToken.mockResolvedValue({
      status: "valid",
      tokenId: "token-1",
      userId: "user-1",
      username: "alice",
      displayName: "Alice",
      avatarUrl: null,
      isAdmin: false,
      expiresAt: null,
    });
    mockState.validateSubmission.mockReturnValue({
      valid: true,
      data: {
        meta: {
          generatedAt: new Date().toISOString(),
          version: "1.0.0",
          sourceId: "machine-a",
          // CLI's default hostname-derived name — the whole point of the
          // test is that the submit path must NOT clobber the user's
          // rename with this.
          sourceName: "CLI on junhoyeo-mbp",
          dateRange: { start: "2024-12-01", end: "2024-12-01" },
        },
        summary: {
          totalTokens: 100,
          totalCost: 0.1,
          totalDays: 1,
          activeDays: 1,
          averagePerDay: 0.1,
          maxCostInSingleDay: 0.1,
          clients: ["claude"],
          models: ["claude-sonnet-4"],
        },
        years: [],
        contributions: [
          {
            date: "2024-12-01",
            totals: { tokens: 100, cost: 0.1, messages: 1 },
            intensity: 1,
            tokenBreakdown: {
              input: 60,
              output: 40,
              cacheRead: 0,
              cacheWrite: 0,
              reasoning: 0,
            },
            clients: [
              {
                client: "claude",
                modelId: "claude-sonnet-4",
                tokens: { input: 60, output: 40, cacheRead: 0, cacheWrite: 0, reasoning: 0 },
                cost: 0.1,
                messages: 1,
              },
            ],
          },
        ],
      },
      errors: [],
      warnings: [],
    });

    // Row is existing source-scoped (not an upgrade) — no rename should flow.
    mockState.resolveSubmissionScope.mockReturnValue({
      kind: "existing",
      submissionId: "submission-1",
      upgradeLegacyRow: false,
    });
    mockState.clientContributionToBreakdownData.mockReturnValue({
      tokens: 100, cost: 0.1, input: 60, output: 40,
      cacheRead: 0, cacheWrite: 0, reasoning: 0, messages: 1,
    });
    mockState.mergeClientBreakdowns.mockImplementation(
      (_existing: unknown, incoming: unknown) => incoming
    );
    mockState.recalculateDayTotals.mockReturnValue({
      tokens: 100, cost: 0.1, inputTokens: 60, outputTokens: 40,
      cacheReadTokens: 0, cacheWriteTokens: 0, reasoningTokens: 0,
    });
    mockState.buildModelBreakdown.mockReturnValue({ "claude-sonnet-4": 100 });
    mockState.mergeTimestampMs.mockReturnValue(null);

    const setCalls: Array<Record<string, unknown>> = [];

    mockState.db.transaction.mockImplementation(async (callback) => {
      const selectResults = [
        [{ id: "submission-1", sourceId: "machine-a" }], // 3a scope
        [], // 3b daily breakdown
        [{
          totalTokens: 100, totalCost: "0.1000",
          inputTokens: 60, outputTokens: 40,
          dateStart: "2024-12-01", dateEnd: "2024-12-01",
          activeDays: 1, rowCount: 1,
        }], // 3d aggregates
        [{ sourceBreakdown: { claude: { tokens: 100, cost: 0.1, input: 60, output: 40,
          cacheRead: 0, cacheWrite: 0, reasoning: 0, messages: 1,
          models: { "claude-sonnet-4": { tokens: 100, cost: 0.1, input: 60, output: 40,
            cacheRead: 0, cacheWrite: 0, reasoning: 0, messages: 1 } } } } }], // 3d allDays
        [{ totalTokens: 100, totalCost: "0.1000",
          dateStart: "2024-12-01", dateEnd: "2024-12-01" }], // metrics
        [{ activeDays: 1 }],
        [{ sourcesUsed: ["claude"] }],
      ];
      const tx = {
        update: vi.fn((table: unknown) => ({
          set: vi.fn((payload: Record<string, unknown>) => {
            if (table === mockState.submissions) setCalls.push(payload);
            return { where: vi.fn(async () => []) };
          }),
        })),
        select: vi.fn(() => {
          const builder = {
            from: vi.fn(() => builder),
            innerJoin: vi.fn(() => builder),
            where: vi.fn(() => builder),
            limit: vi.fn(async () => selectResults.shift() ?? []),
            for: vi.fn(() => builder),
            then: (resolve: (value: unknown) => unknown) =>
              resolve(selectResults.shift() ?? []),
          };
          return builder;
        }),
        insert: vi.fn(() => ({
          values: vi.fn(async () => []),
          onConflictDoNothing: vi.fn(() => ({ returning: vi.fn(async () => []) })),
        })),
        execute: vi.fn(async () => []),
      };
      return callback(tx as never);
    });

    const response = await POST(
      new Request("http://localhost:3000/api/submit", {
        method: "POST",
        headers: {
          Authorization: "Bearer tt_valid",
          "Content-Type": "application/json",
        },
        body: JSON.stringify({ meta: {}, contributions: [] }),
      })
    );

    expect(response.status).toBe(200);
    // The submission-row update must NOT include sourceName — otherwise the
    // CLI's default would clobber a user rename from PATCH /sources/:id.
    const submissionUpdate = setCalls.find(
      (c) => c.totalTokens !== undefined
    );
    expect(submissionUpdate).toBeDefined();
    expect(submissionUpdate).not.toHaveProperty("sourceName");
    // And it must NOT include sourceId either on a non-upgrade path.
    expect(submissionUpdate).not.toHaveProperty("sourceId");
  });

  it("writes sourceId+sourceName when upgrading a legacy unsourced row", async () => {
    mockState.authenticatePersonalToken.mockResolvedValue({
      status: "valid",
      tokenId: "token-1",
      userId: "user-1",
      username: "alice",
      displayName: "Alice",
      avatarUrl: null,
      isAdmin: false,
      expiresAt: null,
    });
    mockState.validateSubmission.mockReturnValue({
      valid: true,
      data: {
        meta: {
          generatedAt: new Date().toISOString(),
          version: "1.0.0",
          sourceId: "machine-a",
          sourceName: "CLI on junhoyeo-mbp",
          dateRange: { start: "2024-12-01", end: "2024-12-01" },
        },
        summary: {
          totalTokens: 100, totalCost: 0.1,
          totalDays: 1, activeDays: 1,
          averagePerDay: 0.1, maxCostInSingleDay: 0.1,
          clients: ["claude"], models: ["claude-sonnet-4"],
        },
        years: [],
        contributions: [{
          date: "2024-12-01",
          totals: { tokens: 100, cost: 0.1, messages: 1 },
          intensity: 1,
          tokenBreakdown: { input: 60, output: 40, cacheRead: 0, cacheWrite: 0, reasoning: 0 },
          clients: [{
            client: "claude", modelId: "claude-sonnet-4",
            tokens: { input: 60, output: 40, cacheRead: 0, cacheWrite: 0, reasoning: 0 },
            cost: 0.1, messages: 1,
          }],
        }],
      },
      errors: [],
      warnings: [],
    });

    mockState.resolveSubmissionScope.mockReturnValue({
      kind: "existing",
      submissionId: "legacy-row",
      upgradeLegacyRow: true,
    });
    mockState.clientContributionToBreakdownData.mockReturnValue({
      tokens: 100, cost: 0.1, input: 60, output: 40,
      cacheRead: 0, cacheWrite: 0, reasoning: 0, messages: 1,
    });
    mockState.mergeClientBreakdowns.mockImplementation(
      (_existing: unknown, incoming: unknown) => incoming
    );
    mockState.recalculateDayTotals.mockReturnValue({
      tokens: 100, cost: 0.1, inputTokens: 60, outputTokens: 40,
      cacheReadTokens: 0, cacheWriteTokens: 0, reasoningTokens: 0,
    });
    mockState.buildModelBreakdown.mockReturnValue({ "claude-sonnet-4": 100 });
    mockState.mergeTimestampMs.mockReturnValue(null);

    const setCalls: Array<Record<string, unknown>> = [];

    mockState.db.transaction.mockImplementation(async (callback) => {
      const selectResults = [
        [{ id: "legacy-row", sourceId: null }], // 3a scope — legacy row
        [],
        [{
          totalTokens: 100, totalCost: "0.1000",
          inputTokens: 60, outputTokens: 40,
          dateStart: "2024-12-01", dateEnd: "2024-12-01",
          activeDays: 1, rowCount: 1,
        }],
        [{ sourceBreakdown: {} }],
        [{ totalTokens: 100, totalCost: "0.1000",
          dateStart: "2024-12-01", dateEnd: "2024-12-01" }],
        [{ activeDays: 1 }],
        [{ sourcesUsed: ["claude"] }],
      ];
      const tx = {
        update: vi.fn((table: unknown) => ({
          set: vi.fn((payload: Record<string, unknown>) => {
            if (table === mockState.submissions) setCalls.push(payload);
            return { where: vi.fn(async () => []) };
          }),
        })),
        select: vi.fn(() => {
          const builder = {
            from: vi.fn(() => builder),
            innerJoin: vi.fn(() => builder),
            where: vi.fn(() => builder),
            limit: vi.fn(async () => selectResults.shift() ?? []),
            for: vi.fn(() => builder),
            then: (resolve: (value: unknown) => unknown) =>
              resolve(selectResults.shift() ?? []),
          };
          return builder;
        }),
        insert: vi.fn(() => ({
          values: vi.fn(async () => []),
          onConflictDoNothing: vi.fn(() => ({ returning: vi.fn(async () => []) })),
        })),
        execute: vi.fn(async () => []),
      };
      return callback(tx as never);
    });

    const response = await POST(
      new Request("http://localhost:3000/api/submit", {
        method: "POST",
        headers: {
          Authorization: "Bearer tt_valid",
          "Content-Type": "application/json",
        },
        body: JSON.stringify({ meta: {}, contributions: [] }),
      })
    );

    expect(response.status).toBe(200);
    const submissionUpdate = setCalls.find((c) => c.totalTokens !== undefined);
    expect(submissionUpdate).toBeDefined();
    // The upgrade path MUST stamp sourceId + the CLI-provided sourceName
    // on the former legacy row.
    expect(submissionUpdate).toMatchObject({
      sourceId: "machine-a",
      sourceName: "CLI on junhoyeo-mbp",
    });
  });
});
