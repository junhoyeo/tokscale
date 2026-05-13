import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

const mockState = vi.hoisted(() => {
  const authenticatePersonalToken = vi.fn();
  const validateSubmission = vi.fn();
  const generateSubmissionHash = vi.fn(() => "submission-hash");
  const revalidateTag = vi.fn();
  const revalidateUsernamePaths = vi.fn();
  const mergeClientBreakdowns = vi.fn();
  const recalculateDayTotals = vi.fn();
  const buildModelBreakdown = vi.fn();
  const clientContributionToBreakdownData = vi.fn();
  const mergeTimestampMs = vi.fn();

  const db = {
    transaction: vi.fn(),
  };

  return {
    authenticatePersonalToken,
    validateSubmission,
    generateSubmissionHash,
    revalidateTag,
    revalidateUsernamePaths,
    mergeClientBreakdowns,
    recalculateDayTotals,
    buildModelBreakdown,
    clientContributionToBreakdownData,
    mergeTimestampMs,
    db,
    reset() {
      authenticatePersonalToken.mockReset();
      validateSubmission.mockReset();
      generateSubmissionHash.mockClear();
      revalidateTag.mockClear();
      revalidateUsernamePaths.mockReset();
      mergeClientBreakdowns.mockReset();
      recalculateDayTotals.mockReset();
      buildModelBreakdown.mockReset();
      clientContributionToBreakdownData.mockReset();
      mergeTimestampMs.mockReset();
      db.transaction.mockReset();
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
  apiTokens: {
    id: "apiTokens.id",
  },
  submissions: {
    id: "submissions.id",
    userId: "submissions.userId",
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
    cliVersion: "submissions.cliVersion",
    submissionHash: "submissions.submissionHash",
    schemaVersion: "submissions.schemaVersion",
  },
  dailyBreakdown: {
    id: "dailyBreakdown.id",
    submissionId: "dailyBreakdown.submissionId",
    date: "dailyBreakdown.date",
    timestampMs: "dailyBreakdown.timestampMs",
    sourceBreakdown: "dailyBreakdown.sourceBreakdown",
    tokens: "dailyBreakdown.tokens",
    cost: "dailyBreakdown.cost",
    inputTokens: "dailyBreakdown.inputTokens",
    outputTokens: "dailyBreakdown.outputTokens",
  },
}));

vi.mock("@/lib/validation/submission", () => ({
  validateSubmission: mockState.validateSubmission,
  generateSubmissionHash: mockState.generateSubmissionHash,
}));

vi.mock("@/lib/db/helpers", () => ({
  mergeClientBreakdowns: mockState.mergeClientBreakdowns,
  recalculateDayTotals: mockState.recalculateDayTotals,
  buildModelBreakdown: mockState.buildModelBreakdown,
  clientContributionToBreakdownData: mockState.clientContributionToBreakdownData,
  mergeTimestampMs: mockState.mergeTimestampMs,
}));

vi.mock("@/lib/db/usernameLookup", () => ({
  normalizeUsernameCacheKey: (username: string) => username.toLowerCase(),
  revalidateUsernamePaths: mockState.revalidateUsernamePaths,
}));

type ModuleExports = typeof import("../../src/app/api/submit/route");

let POST: ModuleExports["POST"];

beforeAll(async () => {
  const routeModule = await import("../../src/app/api/submit/route");
  POST = routeModule.POST;
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

  it("accepts the bearer scheme case-insensitively", async () => {
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
          Authorization: "bearer tt_valid",
          "Content-Type": "application/json",
        },
        body: JSON.stringify({ meta: {}, contributions: [] }),
      })
    );

    expect(response.status).toBe(400);
    expect(mockState.authenticatePersonalToken).toHaveBeenCalledWith("tt_valid", {
      touchLastUsedAt: false,
    });
  });

  it("revalidates username ISR paths after a successful submit", async () => {
    mockState.authenticatePersonalToken.mockResolvedValue({
      status: "valid",
      tokenId: "token-1",
      userId: "user-1",
      username: "Alice",
      displayName: "Alice",
      avatarUrl: null,
      isAdmin: false,
      expiresAt: null,
    });

    mockState.validateSubmission.mockReturnValue({
      valid: true,
      data: {
        meta: {
          version: "2.0.0",
          dateRange: { start: "2026-04-30", end: "2026-04-30" },
        },
        summary: {
          clients: ["codex"],
        },
        contributions: [
          {
            date: "2026-04-30",
            timestampMs: 123,
            clients: [
              {
                client: "codex",
                modelId: "gpt-5.5",
                tokens: 12,
                cost: 0.5,
                input: 7,
                output: 5,
                cacheRead: 0,
                cacheWrite: 0,
                reasoning: 0,
                messages: 1,
              },
            ],
          },
        ],
      },
      errors: [],
      warnings: [],
    });

    mockState.clientContributionToBreakdownData.mockReturnValue({
      tokens: 12,
      cost: 0.5,
      input: 7,
      output: 5,
      cacheRead: 0,
      cacheWrite: 0,
      reasoning: 0,
      messages: 1,
    });
    mockState.recalculateDayTotals.mockReturnValue({
      tokens: 12,
      cost: 0.5,
      inputTokens: 7,
      outputTokens: 5,
    });
    mockState.buildModelBreakdown.mockReturnValue({ "gpt-5.5": 12 });
    mockState.mergeTimestampMs.mockImplementation((_existing: unknown, incoming: unknown) => incoming);

    const selectResults = [
      [],
      [],
      [{
        totalTokens: 12,
        totalCost: "0.5000",
        inputTokens: 7,
        outputTokens: 5,
        dateStart: "2026-04-30",
        dateEnd: "2026-04-30",
        activeDays: 1,
        rowCount: 1,
      }],
      [{
        sourceBreakdown: {
          codex: {
            cacheRead: 0,
            cacheWrite: 0,
            reasoning: 0,
            modelId: "gpt-5.5",
            models: { "gpt-5.5": { tokens: 12 } },
          },
        },
      }],
    ];

    function makeAwaitableBuilder(result: unknown) {
      const builder = {
        from: vi.fn(() => builder),
        where: vi.fn(() => builder),
        for: vi.fn(() => builder),
        limit: vi.fn(() => builder),
        then: (resolve: (value: unknown) => unknown) => Promise.resolve(resolve(result)),
      };
      return builder;
    }

    let insertCall = 0;
    const tx = {
      update: vi.fn(() => {
        const builder = {
          set: vi.fn(() => builder),
          where: vi.fn(() => Promise.resolve()),
        };
        return builder;
      }),
      select: vi.fn(() => makeAwaitableBuilder(selectResults.shift() ?? [])),
      insert: vi.fn(() => {
        insertCall += 1;
        if (insertCall === 1) {
          const builder = {
            values: vi.fn(() => builder),
            returning: vi.fn(() => Promise.resolve([{ id: "submission-1" }])),
          };
          return builder;
        }

        return {
          values: vi.fn(() => Promise.resolve()),
        };
      }),
      execute: vi.fn(() => Promise.resolve()),
    };
    type MockTransaction = typeof tx;

    mockState.db.transaction.mockImplementation(async (callback: (tx: MockTransaction) => Promise<unknown>) =>
      callback(tx)
    );

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
    expect(mockState.revalidateTag).toHaveBeenNthCalledWith(1, "leaderboard", "max");
    expect(mockState.revalidateTag).toHaveBeenNthCalledWith(2, "user:alice", "max");
    expect(mockState.revalidateTag).toHaveBeenNthCalledWith(3, "user-rank", "max");
    expect(mockState.revalidateTag).toHaveBeenNthCalledWith(4, "user-rank:alice", "max");
    expect(mockState.revalidateUsernamePaths).toHaveBeenCalledWith("Alice");
  });
});
