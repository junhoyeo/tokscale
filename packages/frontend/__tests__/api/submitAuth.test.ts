import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import type * as SubmissionTrustModule from "../../src/lib/validation/submissionTrust";
import { PgDialect } from "drizzle-orm/pg-core";

const mockState = vi.hoisted(() => {
  const authenticatePersonalToken = vi.fn();
  const validateSubmission = vi.fn();
  const generateSubmissionHash = vi.fn(() => "submission-hash");
  const assessSubmissionTrust = vi.fn();
  const revalidateTag = vi.fn();
  const revalidateUsernamePaths = vi.fn();
  const revalidateUserGroupLeaderboards = vi.fn();
  const mergeClientBreakdowns = vi.fn();
  const mergeClientBreakdownsWithRegressionGuard = vi.fn();
  const recalculateDayTotals = vi.fn();
  const deriveClientBreakdownProvenance = vi.fn((breakdown) => ({
    schemaVersion: 1,
    messageCount: breakdown.messages ?? 0,
    modelCount: breakdown.models ? Object.keys(breakdown.models).length : 0,
  }));
  const clientContributionToBreakdownData = vi.fn();
  const mergeTimestampMs = vi.fn();
  const revalidatePath = vi.fn();

  const db = {
    transaction: vi.fn(),
  };

  return {
    authenticatePersonalToken,
    validateSubmission,
    generateSubmissionHash,
    assessSubmissionTrust,
    revalidateTag,
    revalidateUsernamePaths,
    revalidateUserGroupLeaderboards,
    mergeClientBreakdowns,
    mergeClientBreakdownsWithRegressionGuard,
    recalculateDayTotals,
    deriveClientBreakdownProvenance,
    clientContributionToBreakdownData,
    mergeTimestampMs,
    revalidatePath,
    db,
    reset() {
      authenticatePersonalToken.mockReset();
      validateSubmission.mockReset();
      generateSubmissionHash.mockClear();
      assessSubmissionTrust.mockReset();
      assessSubmissionTrust.mockReturnValue({
        trustState: "trusted",
        reasonCodes: [],
        rejectionReasonCodes: [],
        reviewDates: [],
        errors: [],
        warnings: [],
      });
      revalidateTag.mockClear();
      revalidateUsernamePaths.mockReset();
      revalidateUserGroupLeaderboards.mockReset();
      mergeClientBreakdowns.mockReset();
      mergeClientBreakdownsWithRegressionGuard.mockReset();
      recalculateDayTotals.mockReset();
      deriveClientBreakdownProvenance.mockClear();
      clientContributionToBreakdownData.mockReset();
      mergeTimestampMs.mockReset();
      revalidatePath.mockClear();
      db.transaction.mockReset();
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
    submitCount: "submissions.submitCount",
    metadataReceivedAt: "submissions.metadataReceivedAt",
    schemaVersion: "submissions.schemaVersion",
  },
  submissionReviews: {
    id: "submissionReviews.id",
    userId: "submissionReviews.userId",
    submissionHash: "submissionReviews.submissionHash",
    trustState: "submissionReviews.trustState",
    competitiveWriteApplied: "submissionReviews.competitiveWriteApplied",
  },
  submittedDevices: {
    id: "submittedDevices.id",
    userId: "submittedDevices.userId",
    deviceKey: "submittedDevices.deviceKey",
    displayName: "submittedDevices.displayName",
    lastSubmittedAt: "submittedDevices.lastSubmittedAt",
    updatedAt: "submittedDevices.updatedAt",
  },
  dailyBreakdown: {
    id: "dailyBreakdown.id",
    submissionId: "dailyBreakdown.submissionId",
    submittedDeviceId: "dailyBreakdown.submittedDeviceId",
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

vi.mock(
  "@/lib/validation/submissionTrust",
  async (importOriginal) => {
    const actual = await importOriginal<typeof SubmissionTrustModule>();
    return {
      ...actual,
      assessSubmissionTrust: mockState.assessSubmissionTrust,
    };
  }
);

vi.mock("@/lib/db/helpers", () => ({
  mergeClientBreakdowns: mockState.mergeClientBreakdowns,
  mergeClientBreakdownsWithRegressionGuard: mockState.mergeClientBreakdownsWithRegressionGuard,
  recalculateDayTotals: mockState.recalculateDayTotals,
  deriveClientBreakdownProvenance: mockState.deriveClientBreakdownProvenance,
  clientContributionToBreakdownData: mockState.clientContributionToBreakdownData,
  mergeTimestampMs: mockState.mergeTimestampMs,
}));

vi.mock("@/lib/db/usernameLookup", () => ({
  normalizeUsernameCacheKey: (username: string) => username.toLowerCase(),
  revalidateUsernamePaths: mockState.revalidateUsernamePaths,
}));

vi.mock("@/lib/groups/cache", () => ({
  revalidateUserGroupLeaderboards: mockState.revalidateUserGroupLeaderboards,
}));

let POST: (request: Request) => Promise<Response>;

beforeAll(async () => {
  const routeModule = await import("../../src/app/api/submit/route");
  POST = routeModule.POST;
});

beforeEach(() => {
  mockState.reset();
});

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
    expect(mockState.revalidateTag).not.toHaveBeenCalled();
    expect(mockState.revalidateUsernamePaths).not.toHaveBeenCalled();
    expect(await response.json()).toEqual({
      error: "Validation failed",
      details: ["bad payload"],
      trustState: "rejected",
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

  it("returns validation errors for a null JSON body without entering the transaction path", async () => {
    mockState.authenticatePersonalToken.mockResolvedValue({
      status: "valid",
      tokenId: "token-1",
      userId: "user-1",
      username: "alice",
      displayName: "Alice",
      avatarUrl: null,
      expiresAt: null,
    });
    mockState.validateSubmission.mockReturnValue({
      valid: false,
      data: null,
      errors: ["Submission data must be an object"],
    });

    const response = await POST(
      new Request("http://localhost:3000/api/submit", {
        method: "POST",
        headers: {
          Authorization: "Bearer tt_valid",
          "Content-Type": "application/json",
        },
        body: "null",
      })
    );

    expect(response.status).toBe(400);
    expect(mockState.validateSubmission).toHaveBeenCalledWith(null);
    expect(mockState.db.transaction).not.toHaveBeenCalled();
    expect(await response.json()).toEqual({
      error: "Validation failed",
      details: ["Submission data must be an object"],
      trustState: "rejected",
    });
  });

  it("rejects oversized submission bodies before JSON parsing", async () => {
    mockState.authenticatePersonalToken.mockResolvedValue({
      status: "valid",
      tokenId: "token-1",
      userId: "user-1",
      username: "alice",
      displayName: "Alice",
      avatarUrl: null,
      expiresAt: null,
    });

    const response = await POST(
      new Request("http://localhost:3000/api/submit", {
        method: "POST",
        headers: {
          Authorization: "Bearer tt_valid",
          "Content-Type": "application/json",
          "Content-Length": String(5 * 1024 * 1024 + 1),
        },
        body: "{}",
      })
    );

    expect(response.status).toBe(413);
    expect(await response.json()).toEqual({
      error: "Submission body is too large",
    });
    const untrustedLengthRequest = new Request(
      "http://localhost:3000/api/submit",
      {
        method: "POST",
        headers: {
          Authorization: "Bearer tt_valid",
          "Content-Type": "application/json",
        },
        body: " ".repeat(5 * 1024 * 1024 + 1),
      }
    );
    expect(untrustedLengthRequest.headers.get("Content-Length")).toBeNull();
    const streamedResponse = await POST(untrustedLengthRequest);
    expect(streamedResponse.status).toBe(413);
    expect(await streamedResponse.json()).toEqual({
      error: "Submission body is too large",
    });
    expect(mockState.validateSubmission).not.toHaveBeenCalled();
    expect(mockState.db.transaction).not.toHaveBeenCalled();
  });

  it("rejects oversized or control-bearing MCP metadata", async () => {
    mockState.authenticatePersonalToken.mockResolvedValue({
      status: "valid",
      tokenId: "token-1",
      userId: "user-1",
      username: "alice",
      displayName: "Alice",
      avatarUrl: null,
      expiresAt: null,
    });

    const oversizedResponse = await POST(
      new Request("http://localhost:3000/api/submit", {
        method: "POST",
        headers: {
          Authorization: "Bearer tt_valid",
          "Content-Type": "application/json",
        },
        body: JSON.stringify({
          mcpServers: Array.from({ length: 101 }, (_, index) => `server-${index}`),
        }),
      })
    );
    const controlResponse = await POST(
      new Request("http://localhost:3000/api/submit", {
        method: "POST",
        headers: {
          Authorization: "Bearer tt_valid",
          "Content-Type": "application/json",
        },
        body: JSON.stringify({ mcpServers: ["server\u001b[31m"] }),
      })
    );

    expect(oversizedResponse.status).toBe(400);
    expect(await oversizedResponse.json()).toEqual({
      error: "Invalid MCP server metadata",
    });
    expect(controlResponse.status).toBe(400);
    expect(await controlResponse.json()).toEqual({
      error: "Invalid MCP server metadata",
    });
    expect(mockState.validateSubmission).not.toHaveBeenCalled();
    expect(mockState.db.transaction).not.toHaveBeenCalled();
  });

  it("revalidates username ISR paths after a successful submit", async () => {
    mockState.authenticatePersonalToken.mockResolvedValue({
      status: "valid",
      tokenId: "token-1",
      userId: "user-1",
      username: "Alice",
      displayName: "Alice",
      avatarUrl: null,
      expiresAt: null,
    });

    mockState.validateSubmission.mockReturnValue({
      valid: true,
      data: {
        device: {
          id: "dev_test",
          name: "Test device",
        },
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
    mockState.mergeTimestampMs.mockImplementation((_existing: unknown, incoming: unknown) => incoming);
    mockState.revalidateUserGroupLeaderboards.mockRejectedValueOnce(
      new Error("group cache unavailable")
    );

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

    let insertCall = 0;
    let submittedDeviceValues: unknown;
    let dailyInsertValues: unknown;
    let submissionUpdateValues: unknown;
    const tx = {
      update: vi.fn((table: unknown) => {
        const builder = {
          set: vi.fn((values: unknown) => {
            if (
              table &&
              typeof table === "object" &&
              (table as { userId?: unknown }).userId === "submissions.userId"
            ) {
              submissionUpdateValues = values;
            }
            return builder;
          }),
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

        if (insertCall === 2) {
          const builder = {
            values: vi.fn((values: unknown) => {
              submittedDeviceValues = values;
              return builder;
            }),
            onConflictDoUpdate: vi.fn(() => builder),
            returning: vi.fn(() => Promise.resolve([{ id: "submitted-device-1" }])),
          };
          return builder;
        }

        return {
          values: vi.fn((values: unknown) => {
            dailyInsertValues = values;
            return Promise.resolve();
          }),
        };
      }),
      execute: vi.fn(() => Promise.resolve()),
      // Nested transaction (Postgres SAVEPOINT). Mock just invokes the
      // callback with the same tx so calls inside the savepoint still
      // count toward tx.execute / tx.update / etc.
      transaction: vi.fn(async (callback: (sp: typeof tx) => Promise<unknown>) =>
        callback(tx)
      ),
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
        body: JSON.stringify({
          meta: {},
          contributions: [],
          mcpServers: ["github", "", "slack", 123, null],
        }),
      })
    );

    expect(response.status).toBe(200);
    expect(tx.insert).toHaveBeenNthCalledWith(2, expect.objectContaining({
      id: "submittedDevices.id",
    }));
    expect(submittedDeviceValues).toEqual(expect.objectContaining({
      userId: "user-1",
      deviceKey: "dev_test",
      displayName: "Test device",
    }));
    expect(dailyInsertValues).toEqual([
      expect.objectContaining({
        submissionId: "submission-1",
        submittedDeviceId: "submitted-device-1",
        date: "2026-04-30",
      }),
    ]);
    expect(submissionUpdateValues).toEqual(
      expect.objectContaining({
        mcpServers: ["github", "slack"],
        submitCount: 1,
      })
    );
    expect(mockState.revalidateTag).toHaveBeenNthCalledWith(1, "leaderboard", "max");
    expect(mockState.revalidateTag).toHaveBeenNthCalledWith(2, "user:alice", "max");
    expect(mockState.revalidateTag).toHaveBeenNthCalledWith(3, "user-rank", "max");
    expect(mockState.revalidateTag).toHaveBeenNthCalledWith(4, "user-rank:alice", "max");
    expect(mockState.revalidatePath).toHaveBeenCalledTimes(3);
    expect(mockState.revalidatePath).toHaveBeenNthCalledWith(1, "/");
    expect(mockState.revalidatePath).toHaveBeenNthCalledWith(2, "/api/leaderboard");
    expect(mockState.revalidatePath).toHaveBeenNthCalledWith(3, "/leaderboard");
    expect(mockState.revalidateUserGroupLeaderboards).toHaveBeenCalledWith("user-1");
    expect(mockState.revalidateUsernamePaths).toHaveBeenCalledWith("Alice");
  });

  it("replaces same-device rows while preserving newer submission metadata", async () => {
    mockState.authenticatePersonalToken.mockResolvedValue({
      status: "valid",
      tokenId: "token-1",
      userId: "user-1",
      username: "alice",
      displayName: "Alice",
      avatarUrl: null,
      expiresAt: null,
    });

    mockState.validateSubmission.mockReturnValue({
      valid: true,
      data: {
        device: {
          id: "dev_laptop",
        },
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
            timestampMs: 456,
            clients: [
              {
                client: "codex",
                modelId: "gpt-5.5",
                tokens: 15,
                cost: 0.75,
                input: 10,
                output: 5,
                cacheRead: 0,
                cacheWrite: 0,
                reasoning: 0,
                messages: 1,
              },
            ],
          },
        ],
        timeMetrics: {
          longestContinuousMs: 1_000,
          maxConcurrentSessions: 2,
          sessionCount: 3,
        },
      },
      errors: [],
      warnings: [],
    });

    const incomingBreakdown = {
      tokens: 15,
      cost: 0.75,
      input: 10,
      output: 5,
      cacheRead: 0,
      cacheWrite: 0,
      reasoning: 0,
      messages: 1,
    };
    const existingBreakdown = {
      codex: {
        tokens: 12,
        cost: 0.5,
        input: 7,
        output: 5,
        cacheRead: 0,
        cacheWrite: 0,
        reasoning: 0,
        messages: 1,
        models: { "gpt-5.5": { tokens: 12 } },
      },
    };
    const mergedBreakdown = {
      codex: {
        ...incomingBreakdown,
        models: { "gpt-5.5": incomingBreakdown },
      },
    };

    mockState.clientContributionToBreakdownData.mockReturnValue(incomingBreakdown);
    mockState.mergeClientBreakdownsWithRegressionGuard.mockReturnValue({
      merged: mergedBreakdown,
      warnings: [],
    });
    mockState.recalculateDayTotals.mockReturnValue({
      tokens: 15,
      cost: 0.75,
      inputTokens: 10,
      outputTokens: 5,
    });
    mockState.mergeTimestampMs.mockReturnValue(456);

    const selectResults = [
      [{
        id: "submission-1",
        metadataReceivedAt: new Date("2099-01-01T00:00:00.000Z"),
      }],
      [{
        id: "daily-1",
        date: "2026-04-30",
        timestampMs: 123,
        sourceBreakdown: existingBreakdown,
      }],
      [{
        totalTokens: 15,
        totalCost: "0.7500",
        inputTokens: 10,
        outputTokens: 5,
        dateStart: "2026-04-30",
        dateEnd: "2026-04-30",
        activeDays: 1,
        rowCount: 1,
      }],
      [{ sourceBreakdown: mergedBreakdown }],
    ];

    let submissionUpdateValues: Record<string, unknown> | undefined;
    const tx = {
      update: vi.fn((table: unknown) => {
        const builder = {
          set: vi.fn((values: Record<string, unknown>) => {
            if (
              table &&
              typeof table === "object" &&
              (table as { userId?: unknown }).userId === "submissions.userId"
            ) {
              submissionUpdateValues = values;
            }
            return builder;
          }),
          where: vi.fn(() => Promise.resolve()),
        };
        return builder;
      }),
      select: vi.fn(() => makeAwaitableBuilder(selectResults.shift() ?? [])),
      insert: vi.fn(() => {
        const builder = {
          values: vi.fn(() => builder),
          onConflictDoUpdate: vi.fn(() => builder),
          returning: vi.fn(() => Promise.resolve([{ id: "submitted-device-1" }])),
        };
        return builder;
      }),
      execute: vi.fn(() => Promise.resolve()),
      // Nested transaction (Postgres SAVEPOINT). Mock just invokes the
      // callback with the same tx so calls inside the savepoint still
      // count toward tx.execute / tx.update / etc.
      transaction: vi.fn(async (callback: (sp: typeof tx) => Promise<unknown>) =>
        callback(tx)
      ),
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
        body: JSON.stringify({
          meta: {},
          contributions: [],
          mcpServers: ["stale-server"],
        }),
      })
    );

    expect(response.status).toBe(200);
    expect(tx.insert).toHaveBeenCalledTimes(1);
    expect(tx.insert).toHaveBeenCalledWith(expect.objectContaining({
      id: "submittedDevices.id",
    }));
    expect(tx.execute).toHaveBeenCalledTimes(1);
    expect(submissionUpdateValues).toEqual(
      expect.objectContaining({
        totalTokens: 15,
        totalCost: "0.7500",
      })
    );
    expect(submissionUpdateValues).not.toHaveProperty("cliVersion");
    expect(submissionUpdateValues).not.toHaveProperty("submissionHash");
    expect(submissionUpdateValues).not.toHaveProperty("metadataReceivedAt");
    expect(submissionUpdateValues).not.toHaveProperty("longestContinuousMs");
    expect(submissionUpdateValues).not.toHaveProperty("maxConcurrentSessions");
    expect(submissionUpdateValues).not.toHaveProperty("sessionCount");
    expect(submissionUpdateValues).not.toHaveProperty("mcpServers");
    expect(mockState.mergeClientBreakdownsWithRegressionGuard).toHaveBeenCalledWith(
      existingBreakdown,
      {
        codex: {
          ...mergedBreakdown.codex,
          provenance: {
            schemaVersion: 1,
            messageCount: 1,
            modelCount: 1,
          },
        },
      },
      expect.any(Set)
    );
    expect(await response.json()).toEqual(expect.objectContaining({
      success: true,
      metrics: expect.objectContaining({
        totalTokens: 15,
        activeDays: 1,
      }),
      mode: "merge",
    }));
  });

  it("merges after a concurrent first submit creates the submission first", async () => {
    mockState.authenticatePersonalToken.mockResolvedValue({
      status: "valid",
      tokenId: "token-1",
      userId: "user-1",
      username: "alice",
      displayName: "Alice",
      avatarUrl: null,
      expiresAt: null,
    });

    mockState.validateSubmission.mockReturnValue({
      valid: true,
      data: {
        device: {
          id: "dev_laptop",
        },
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
            timestampMs: 456,
            clients: [
              {
                client: "codex",
                modelId: "gpt-5.5",
                tokens: 15,
                cost: 0.75,
                input: 10,
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

    const incomingBreakdown = {
      tokens: 15,
      cost: 0.75,
      input: 10,
      output: 5,
      cacheRead: 0,
      cacheWrite: 0,
      reasoning: 0,
      messages: 1,
    };
    const existingBreakdown = {
      codex: {
        tokens: 12,
        cost: 0.5,
        input: 7,
        output: 5,
        cacheRead: 0,
        cacheWrite: 0,
        reasoning: 0,
        messages: 1,
        models: { "gpt-5.5": { tokens: 12 } },
      },
    };
    const mergedBreakdown = {
      codex: {
        ...incomingBreakdown,
        models: { "gpt-5.5": incomingBreakdown },
      },
    };

    mockState.clientContributionToBreakdownData.mockReturnValue(incomingBreakdown);
    mockState.mergeClientBreakdownsWithRegressionGuard.mockReturnValue({
      merged: mergedBreakdown,
      warnings: [],
    });
    mockState.recalculateDayTotals.mockReturnValue({
      tokens: 15,
      cost: 0.75,
      inputTokens: 10,
      outputTokens: 5,
    });
    mockState.mergeTimestampMs.mockReturnValue(456);

    const selectResults = [
      [],
      [{ id: "submission-1" }],
      [{
        id: "daily-1",
        date: "2026-04-30",
        timestampMs: 123,
        activeTimeMs: null,
        sourceBreakdown: existingBreakdown,
      }],
      [{
        totalTokens: 15,
        totalCost: "0.7500",
        inputTokens: 10,
        outputTokens: 5,
        dateStart: "2026-04-30",
        dateEnd: "2026-04-30",
        activeDays: 1,
        rowCount: 1,
      }],
      [{ sourceBreakdown: mergedBreakdown }],
    ];

    let insertCall = 0;
    let dailyInsertValues: unknown;
    const uniqueSubmissionRace = Object.assign(
      new Error("duplicate key value violates unique constraint"),
      {
        code: "23505",
        constraint: "submissions_user_id_unique",
      }
    );
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
            returning: vi.fn(() => Promise.reject(uniqueSubmissionRace)),
          };
          return builder;
        }

        if (insertCall === 2) {
          const builder = {
            values: vi.fn(() => builder),
            onConflictDoUpdate: vi.fn(() => builder),
            returning: vi.fn(() => Promise.resolve([{ id: "submitted-device-1" }])),
          };
          return builder;
        }

        const builder = {
          values: vi.fn((values: unknown) => {
            dailyInsertValues = values;
            return Promise.resolve();
          }),
        };
        return builder;
      }),
      execute: vi.fn(() => Promise.resolve()),
      transaction: vi.fn(async (callback: (sp: typeof tx) => Promise<unknown>) =>
        callback(tx)
      ),
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
    expect(dailyInsertValues).toBeUndefined();
    expect(tx.execute).toHaveBeenCalledTimes(1);
    expect(mockState.mergeClientBreakdownsWithRegressionGuard).toHaveBeenCalledWith(
      existingBreakdown,
      {
        codex: {
          ...mergedBreakdown.codex,
          provenance: {
            schemaVersion: 1,
            messageCount: 1,
            modelCount: 1,
          },
        },
      },
      expect.any(Set)
    );
    expect(await response.json()).toEqual(expect.objectContaining({
      success: true,
      metrics: expect.objectContaining({
        totalTokens: 15,
        activeDays: 1,
      }),
      mode: "merge",
    }));
  });

  it("sets all-time active time from all submitted device daily rows", async () => {
    mockState.authenticatePersonalToken.mockResolvedValue({
      status: "valid",
      tokenId: "token-1",
      userId: "user-1",
      username: "alice",
      displayName: "Alice",
      avatarUrl: null,
      expiresAt: null,
    });

    mockState.validateSubmission.mockReturnValue({
      valid: true,
      data: {
        device: {
          id: "dev_desktop",
          name: "Desktop",
        },
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
            timestampMs: 456,
            activeTimeMs: 4_000,
            clients: [
              {
                client: "codex",
                modelId: "gpt-5.5",
                tokens: 15,
                cost: 0.75,
                input: 10,
                output: 5,
                cacheRead: 0,
                cacheWrite: 0,
                reasoning: 0,
                messages: 1,
              },
            ],
          },
        ],
        timeMetrics: {
          totalActiveTimeMs: 4_000,
          longestContinuousMs: 4_000,
          maxConcurrentSessions: 1,
          sessionCount: 1,
        },
      },
      errors: [],
      warnings: [],
    });

    const incomingBreakdown = {
      tokens: 15,
      cost: 0.75,
      input: 10,
      output: 5,
      cacheRead: 0,
      cacheWrite: 0,
      reasoning: 0,
      messages: 1,
    };
    const insertedBreakdown = {
      codex: {
        ...incomingBreakdown,
        models: { "gpt-5.5": incomingBreakdown },
        provenance: {
          schemaVersion: 1,
          messageCount: 1,
          modelCount: 1,
        },
      },
    };

    mockState.clientContributionToBreakdownData.mockReturnValue(incomingBreakdown);
    mockState.recalculateDayTotals.mockReturnValue({
      tokens: 15,
      cost: 0.75,
      inputTokens: 10,
      outputTokens: 5,
    });

    const selectResults = [
      [{ id: "submission-1" }],
      [],
      [],
      [{
        totalTokens: 42,
        totalCost: "1.7500",
        inputTokens: 27,
        outputTokens: 15,
        dateStart: "2026-04-30",
        dateEnd: "2026-04-30",
        activeDays: 1,
        rowCount: 3,
        totalActiveTimeMs: 10_000,
      }],
      [{ sourceBreakdown: insertedBreakdown }],
    ];

    const submissionUpdateSets: Array<Record<string, unknown>> = [];
    let insertCall = 0;
    let dailyInsertValues: unknown;
    const tx = {
      update: vi.fn((table: unknown) => {
        const builder = {
          set: vi.fn((values: Record<string, unknown>) => {
            if ((table as { id?: unknown }).id === "submissions.id") {
              submissionUpdateSets.push(values);
            }
            return builder;
          }),
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
            onConflictDoUpdate: vi.fn(() => builder),
            returning: vi.fn(() => Promise.resolve([{ id: "submitted-device-2" }])),
          };
          return builder;
        }

        const builder = {
          values: vi.fn((values: unknown) => {
            dailyInsertValues = values;
            return Promise.resolve();
          }),
        };
        return builder;
      }),
      execute: vi.fn(() => Promise.resolve()),
      transaction: vi.fn(async (callback: (sp: typeof tx) => Promise<unknown>) =>
        callback(tx)
      ),
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
    expect(dailyInsertValues).toEqual([expect.objectContaining({
      activeTimeMs: 4_000,
    })]);
    expect(submissionUpdateSets.at(-1)).toEqual(expect.objectContaining({
      totalActiveTimeMs: 10_000,
      longestContinuousMs: 4_000,
      maxConcurrentSessions: 1,
      sessionCount: 1,
    }));
  });

  it("updates same-device active time totals without double-counting the replaced row", async () => {
    mockState.authenticatePersonalToken.mockResolvedValue({
      status: "valid",
      tokenId: "token-1",
      userId: "user-1",
      username: "alice",
      displayName: "Alice",
      avatarUrl: null,
      expiresAt: null,
    });

    mockState.validateSubmission.mockReturnValue({
      valid: true,
      data: {
        device: {
          id: "dev_laptop",
        },
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
            timestampMs: 456,
            activeTimeMs: 5_000,
            clients: [
              {
                client: "codex",
                modelId: "gpt-5.5",
                tokens: 15,
                cost: 0.75,
                input: 10,
                output: 5,
                cacheRead: 0,
                cacheWrite: 0,
                reasoning: 0,
                messages: 1,
              },
            ],
          },
        ],
        timeMetrics: {
          totalActiveTimeMs: 5_000,
          longestContinuousMs: 5_000,
          maxConcurrentSessions: 1,
          sessionCount: 1,
        },
      },
      errors: [],
      warnings: [],
    });

    const incomingBreakdown = {
      tokens: 15,
      cost: 0.75,
      input: 10,
      output: 5,
      cacheRead: 0,
      cacheWrite: 0,
      reasoning: 0,
      messages: 1,
    };
    const existingBreakdown = {
      codex: {
        tokens: 12,
        cost: 0.5,
        input: 7,
        output: 5,
        cacheRead: 0,
        cacheWrite: 0,
        reasoning: 0,
        messages: 1,
        models: { "gpt-5.5": { tokens: 12 } },
      },
    };
    const mergedBreakdown = {
      codex: {
        ...incomingBreakdown,
        models: { "gpt-5.5": incomingBreakdown },
      },
    };

    mockState.clientContributionToBreakdownData.mockReturnValue(incomingBreakdown);
    mockState.mergeClientBreakdownsWithRegressionGuard.mockReturnValue({
      merged: mergedBreakdown,
      warnings: [],
    });
    mockState.recalculateDayTotals.mockReturnValue({
      tokens: 15,
      cost: 0.75,
      inputTokens: 10,
      outputTokens: 5,
    });
    mockState.mergeTimestampMs.mockReturnValue(456);

    const selectResults = [
      [{ id: "submission-1" }],
      [{
        id: "daily-1",
        date: "2026-04-30",
        timestampMs: 123,
        activeTimeMs: 2_000,
        sourceBreakdown: existingBreakdown,
      }],
      [{
        totalTokens: 27,
        totalCost: "1.2500",
        inputTokens: 17,
        outputTokens: 10,
        dateStart: "2026-04-30",
        dateEnd: "2026-04-30",
        activeDays: 1,
        rowCount: 2,
        totalActiveTimeMs: 13_000,
      }],
      [{ sourceBreakdown: mergedBreakdown }],
    ];

    const submissionUpdateSets: Array<Record<string, unknown>> = [];
    const tx = {
      update: vi.fn((table: unknown) => {
        const builder = {
          set: vi.fn((values: Record<string, unknown>) => {
            if ((table as { id?: unknown }).id === "submissions.id") {
              submissionUpdateSets.push(values);
            }
            return builder;
          }),
          where: vi.fn(() => Promise.resolve()),
        };
        return builder;
      }),
      select: vi.fn(() => makeAwaitableBuilder(selectResults.shift() ?? [])),
      insert: vi.fn(() => {
        const builder = {
          values: vi.fn(() => builder),
          onConflictDoUpdate: vi.fn(() => builder),
          returning: vi.fn(() => Promise.resolve([{ id: "submitted-device-1" }])),
        };
        return builder;
      }),
      execute: vi.fn(() => Promise.resolve()),
      transaction: vi.fn(async (callback: (sp: typeof tx) => Promise<unknown>) =>
        callback(tx)
      ),
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
    expect(tx.insert).toHaveBeenCalledTimes(1);
    expect(tx.execute).toHaveBeenCalledTimes(1);
    expect(submissionUpdateSets.at(-1)).toEqual(expect.objectContaining({
      totalActiveTimeMs: 13_000,
    }));
  });
  it("adopts legacy daily rows into the first modern device instead of duplicating totals", async () => {
    mockState.authenticatePersonalToken.mockResolvedValue({
      status: "valid",
      tokenId: "token-1",
      userId: "user-1",
      username: "alice",
      displayName: "Alice",
      avatarUrl: null,
      expiresAt: null,
    });

    mockState.validateSubmission.mockReturnValue({
      valid: true,
      data: {
        device: {
          id: "dev_laptop",
          name: "Laptop",
        },
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
            timestampMs: 456,
            clients: [
              {
                client: "codex",
                modelId: "gpt-5.5",
                tokens: 15,
                cost: 0.75,
                input: 10,
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

    const incomingBreakdown = {
      tokens: 15,
      cost: 0.75,
      input: 10,
      output: 5,
      cacheRead: 0,
      cacheWrite: 0,
      reasoning: 0,
      messages: 1,
    };
    const legacyBreakdown = {
      codex: {
        tokens: 12,
        cost: 0.5,
        input: 7,
        output: 5,
        cacheRead: 0,
        cacheWrite: 0,
        reasoning: 0,
        messages: 1,
        models: { "gpt-5.5": { tokens: 12 } },
      },
    };
    const mergedBreakdown = {
      codex: {
        ...incomingBreakdown,
        models: { "gpt-5.5": incomingBreakdown },
      },
    };
    const incomingBreakdownWithProvenance = {
      codex: {
        ...mergedBreakdown.codex,
        provenance: {
          schemaVersion: 1,
          messageCount: 1,
          modelCount: 1,
        },
      },
    };

    mockState.clientContributionToBreakdownData.mockReturnValue(incomingBreakdown);
    mockState.mergeClientBreakdownsWithRegressionGuard.mockReturnValue({
      merged: mergedBreakdown,
      warnings: [],
    });
    mockState.recalculateDayTotals.mockReturnValue({
      tokens: 15,
      cost: 0.75,
      inputTokens: 10,
      outputTokens: 5,
    });
    mockState.mergeTimestampMs.mockReturnValue(123);

    const selectResults = [
      [{ id: "submission-1" }],
      [],
      [{
        id: "daily-legacy",
        date: "2026-04-30",
        timestampMs: 123,
        activeTimeMs: null,
        sourceBreakdown: legacyBreakdown,
      }],
      [{
        totalTokens: 15,
        totalCost: "0.7500",
        inputTokens: 10,
        outputTokens: 5,
        dateStart: "2026-04-30",
        dateEnd: "2026-04-30",
        activeDays: 1,
        rowCount: 1,
      }],
      [{ sourceBreakdown: mergedBreakdown }],
    ];

    let insertCall = 0;
    let dailyInsertValues: unknown;
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
            onConflictDoUpdate: vi.fn(() => builder),
            returning: vi.fn(() => Promise.resolve([{ id: "submitted-device-1" }])),
          };
          return builder;
        }

        const builder = {
          values: vi.fn((values: unknown) => {
            dailyInsertValues = values;
            return Promise.resolve();
          }),
        };
        return builder;
      }),
      execute: vi.fn(() => Promise.resolve()),
      // Nested transaction (Postgres SAVEPOINT). Mock just invokes the
      // callback with the same tx so calls inside the savepoint still
      // count toward tx.execute / tx.update / etc.
      transaction: vi.fn(async (callback: (sp: typeof tx) => Promise<unknown>) =>
        callback(tx)
      ),
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
    expect(tx.insert).toHaveBeenCalledTimes(1);
    expect(dailyInsertValues).toBeUndefined();
    expect(tx.execute).toHaveBeenCalledTimes(2);
    expect(mockState.mergeClientBreakdownsWithRegressionGuard).toHaveBeenCalledWith(
      legacyBreakdown,
      incomingBreakdownWithProvenance,
      expect.any(Set)
    );
    expect(await response.json()).toEqual(expect.objectContaining({
      success: true,
      metrics: expect.objectContaining({
        totalTokens: 15,
        activeDays: 1,
      }),
      mode: "merge",
    }));
  });

  it("keeps legacy daily rows separate when another modern device already submitted", async () => {
    mockState.authenticatePersonalToken.mockResolvedValue({
      status: "valid",
      tokenId: "token-1",
      userId: "user-1",
      username: "alice",
      displayName: "Alice",
      avatarUrl: null,
      expiresAt: null,
    });

    mockState.validateSubmission.mockReturnValue({
      valid: true,
      data: {
        device: {
          id: "dev_phone",
          name: "Phone",
        },
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
            timestampMs: 789,
            clients: [
              {
                client: "codex",
                modelId: "gpt-5.5",
                tokens: 15,
                cost: 0.75,
                input: 10,
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

    const incomingBreakdown = {
      tokens: 15,
      cost: 0.75,
      input: 10,
      output: 5,
      cacheRead: 0,
      cacheWrite: 0,
      reasoning: 0,
      messages: 1,
    };
    const insertedBreakdown = {
      codex: {
        ...incomingBreakdown,
        models: { "gpt-5.5": incomingBreakdown },
        provenance: {
          schemaVersion: 1,
          messageCount: 1,
          modelCount: 1,
        },
      },
    };

    mockState.clientContributionToBreakdownData.mockReturnValue(incomingBreakdown);
    mockState.recalculateDayTotals.mockReturnValue({
      tokens: 15,
      cost: 0.75,
      inputTokens: 10,
      outputTokens: 5,
    });

    const selectResults = [
      [{ id: "submission-1" }],
      [],
      [],
      [{
        totalTokens: 42,
        totalCost: "1.7500",
        inputTokens: 27,
        outputTokens: 15,
        dateStart: "2026-04-30",
        dateEnd: "2026-04-30",
        activeDays: 1,
        rowCount: 3,
      }],
      [{ sourceBreakdown: insertedBreakdown }],
    ];

    let insertCall = 0;
    let dailyInsertValues: unknown;
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
            onConflictDoUpdate: vi.fn(() => builder),
            returning: vi.fn(() => Promise.resolve([{ id: "submitted-device-2" }])),
          };
          return builder;
        }

        const builder = {
          values: vi.fn((values: unknown) => {
            dailyInsertValues = values;
            return Promise.resolve();
          }),
        };
        return builder;
      }),
      execute: vi.fn(() => Promise.resolve()),
      // Nested transaction (Postgres SAVEPOINT). Mock just invokes the
      // callback with the same tx so calls inside the savepoint still
      // count toward tx.execute / tx.update / etc.
      transaction: vi.fn(async (callback: (sp: typeof tx) => Promise<unknown>) =>
        callback(tx)
      ),
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
    expect(tx.insert).toHaveBeenCalledTimes(2);
    expect(tx.execute).toHaveBeenCalledTimes(1);
    expect(dailyInsertValues).toEqual([expect.objectContaining({
      submittedDeviceId: "submitted-device-2",
      date: "2026-04-30",
      tokens: 15,
      sourceBreakdown: insertedBreakdown,
    })]);
    expect(mockState.mergeClientBreakdownsWithRegressionGuard).not.toHaveBeenCalled();
    expect(await response.json()).toEqual(expect.objectContaining({
      success: true,
      metrics: expect.objectContaining({
        totalTokens: 42,
        activeDays: 1,
      }),
      mode: "merge",
    }));
  });

  it("hard-rejects trust failures before opening a transaction", async () => {
    mockState.authenticatePersonalToken.mockResolvedValue({
      status: "valid",
      tokenId: "token-1",
      userId: "user-1",
      username: "alice",
      displayName: "Alice",
      avatarUrl: null,
      expiresAt: null,
    });
    mockState.validateSubmission.mockReturnValue({
      valid: true,
      data: {
        summary: { clients: ["codex"] },
        contributions: [{ date: "2026-07-10", clients: [] }],
      },
      errors: [],
      warnings: [],
    });
    mockState.assessSubmissionTrust.mockReturnValue({
      trustState: "rejected",
      reasonCodes: [],
      rejectionReasonCodes: ["timestamp_day_mismatch"],
      reviewDates: [],
      errors: ["timestamp is outside its claimed UTC bucket"],
      warnings: [],
    });

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

    expect(response.status).toBe(400);
    expect(await response.json()).toEqual({
      error: "Submission rejected by trust policy",
      details: ["timestamp is outside its claimed UTC bucket"],
      trustState: "rejected",
      errorCodes: ["timestamp_day_mismatch"],
    });
    expect(mockState.db.transaction).not.toHaveBeenCalled();
    expect(mockState.revalidateTag).not.toHaveBeenCalled();
    expect(mockState.revalidatePath).not.toHaveBeenCalled();
    expect(mockState.revalidateUserGroupLeaderboards).not.toHaveBeenCalled();
    expect(mockState.revalidateUsernamePaths).not.toHaveBeenCalled();
  });

  it("queues an all-review payload without touching competitive tables", async () => {
    mockState.authenticatePersonalToken.mockResolvedValue({
      status: "valid",
      tokenId: "token-1",
      userId: "user-1",
      username: "alice",
      displayName: "Alice",
      avatarUrl: null,
      expiresAt: null,
    });
    const oldDay = {
      date: "2026-01-01",
      totals: { tokens: 100, cost: 0.25, messages: 1 },
      intensity: 1,
      tokenBreakdown: {
        input: 100,
        output: 0,
        cacheRead: 0,
        cacheWrite: 0,
        reasoning: 0,
      },
      clients: [{
        client: "codex",
        modelId: "gpt-5.5",
        tokens: {
          input: 100,
          output: 0,
          cacheRead: 0,
          cacheWrite: 0,
          reasoning: 0,
        },
        cost: 0.25,
        messages: 1,
      }],
    };
    mockState.validateSubmission.mockReturnValue({
      valid: true,
      data: {
        meta: {
          generatedAt: "2026-07-14T00:00:00.000Z",
          version: "2.1.1",
          dateRange: { start: oldDay.date, end: oldDay.date },
        },
        device: {
          id: "device-review",
          name: "Review laptop",
        },
        summary: {
          totalTokens: 100,
          totalCost: 0.25,
          totalDays: 1,
          activeDays: 1,
          averagePerDay: 100,
          maxCostInSingleDay: 0.25,
          clients: ["codex"],
          models: ["gpt-5.5"],
        },
        years: [{
          year: "2026",
          totalTokens: 100,
          totalCost: 0.25,
          range: { start: oldDay.date, end: oldDay.date },
        }],
        contributions: [oldDay],
      },
      errors: [],
      warnings: [],
    });
    mockState.assessSubmissionTrust.mockReturnValue({
      trustState: "review_required",
      reasonCodes: ["historical_day_missing_timestamp"],
      rejectionReasonCodes: [],
      reviewDates: [oldDay.date],
      errors: [],
      warnings: ["old day needs review"],
    });

    let reviewValues: Record<string, unknown> | undefined;
    type ReviewConflictConfig = {
      set: { competitiveWriteApplied: Parameters<PgDialect["sqlToQuery"]>[0] };
    };
    const reviewBuilder = {
      values: vi.fn((values: Record<string, unknown>) => {
        reviewValues = values;
        return reviewBuilder;
      }),
      onConflictDoUpdate: vi.fn((config: ReviewConflictConfig) => {
        if (!config) {
          throw new Error("Expected the review upsert configuration");
        }
        return reviewBuilder;
      }),
      returning: vi.fn(() => Promise.resolve([{ id: "review-1" }])),
    };
    const tx = {
      update: vi.fn(() => {
        const builder = {
          set: vi.fn(() => builder),
          where: vi.fn(() => Promise.resolve()),
        };
        return builder;
      }),
      select: vi.fn(() =>
        makeAwaitableBuilder([{ pendingCount: 0, matchingHashCount: 0 }])
      ),
      insert: vi.fn(() => reviewBuilder),
      execute: vi.fn(() => Promise.resolve()),
    };
    type ReviewOnlyTransaction = typeof tx;
    mockState.db.transaction.mockImplementation(
      async (
        callback: (transaction: ReviewOnlyTransaction) => Promise<unknown>
      ) => callback(tx)
    );

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
    const body = await response.json();

    expect(response.status).toBe(200);
    expect(body).toEqual(
      expect.objectContaining({
        trustState: "review_required",
        submissionId: null,
        reviewId: "review-1",
        reviewMetrics: expect.objectContaining({
          totalTokens: 100,
          totalCost: 0.25,
          activeDays: 1,
        }),
        mode: "review",
        competitiveWriteApplied: false,
      })
    );
    expect(body).not.toHaveProperty("metrics");
    expect(tx.insert).toHaveBeenCalledTimes(1);
    expect(reviewBuilder.onConflictDoUpdate).toHaveBeenCalled();
    expect(tx.execute.mock.invocationCallOrder[0]).toBeLessThan(
      tx.select.mock.invocationCallOrder[0]
    );
    expect(reviewValues).toEqual(
      expect.objectContaining({
        totalTokens: 100,
        totalCost: "0.2500",
        schemaVersion: 2,
        payload: expect.objectContaining({
          device: {
            id: "device-review",
            name: "Review laptop",
          },
        }),
      })
    );
    tx.select.mockReturnValueOnce(
      makeAwaitableBuilder([{ pendingCount: 20, matchingHashCount: 0 }])
    );
    const limitedResponse = await POST(
      new Request("http://localhost:3000/api/submit", {
        method: "POST",
        headers: {
          Authorization: "Bearer tt_valid",
          "Content-Type": "application/json",
        },
        body: JSON.stringify({}),
      })
    );
    expect(limitedResponse.status).toBe(429);
    expect(await limitedResponse.json()).toEqual({
      error: "Pending submission review limit reached",
      trustState: "review_required",
    });
    expect(tx.insert).toHaveBeenCalledTimes(1);
    tx.select.mockReturnValueOnce(
      makeAwaitableBuilder([{ pendingCount: 20, matchingHashCount: 1 }])
    );
    const matchingResponse = await POST(
      new Request("http://localhost:3000/api/submit", {
        method: "POST",
        headers: {
          Authorization: "Bearer tt_valid",
          "Content-Type": "application/json",
        },
        body: JSON.stringify({}),
      })
    );
    expect(matchingResponse.status).toBe(200);
    expect(await matchingResponse.json()).toEqual(
      expect.objectContaining({ reviewId: "review-1" })
    );
    expect(tx.insert).toHaveBeenCalledTimes(2);
    const retryUpsert =
      reviewBuilder.onConflictDoUpdate.mock.calls[1]?.[0];
    if (!retryUpsert) {
      throw new Error("Expected the review retry upsert configuration");
    }
    const compiledWriteMarker = new PgDialect().sqlToQuery(
      retryUpsert.set.competitiveWriteApplied
    );
    expect(compiledWriteMarker.sql).toBe("$1 OR $2");
    expect(compiledWriteMarker.params).toEqual([
      "submissionReviews.competitiveWriteApplied",
      false,
    ]);
    expect(mockState.revalidateTag).not.toHaveBeenCalled();
    expect(mockState.revalidatePath).not.toHaveBeenCalled();
    expect(mockState.revalidateUserGroupLeaderboards).not.toHaveBeenCalled();
    expect(mockState.revalidateUsernamePaths).not.toHaveBeenCalled();
  });

  it("queues only flagged days while persisting trusted days in one transaction", async () => {
    mockState.authenticatePersonalToken.mockResolvedValue({
      status: "valid",
      tokenId: "token-1",
      userId: "user-1",
      username: "alice",
      displayName: "Alice",
      avatarUrl: null,
      expiresAt: null,
    });

    const oldDay = {
      date: "2026-01-01",
      totals: { tokens: 400, cost: 1, messages: 1 },
      intensity: 1,
      tokenBreakdown: {
        input: 400,
        output: 0,
        cacheRead: 0,
        cacheWrite: 0,
        reasoning: 0,
      },
      clients: [{
        client: "codex",
        modelId: "gpt-5.5",
        tokens: {
          input: 400,
          output: 0,
          cacheRead: 0,
          cacheWrite: 0,
          reasoning: 0,
        },
        cost: 1,
        messages: 1,
      }],
    };
    const trustedDay = {
      date: "2026-07-10",
      timestampMs: Date.parse("2026-07-10T12:00:00.000Z"),
      totals: { tokens: 100, cost: 0.25, messages: 1 },
      intensity: 1,
      tokenBreakdown: {
        input: 100,
        output: 0,
        cacheRead: 0,
        cacheWrite: 0,
        reasoning: 0,
      },
      clients: [{
        client: "codex",
        modelId: "gpt-5.5",
        tokens: {
          input: 100,
          output: 0,
          cacheRead: 0,
          cacheWrite: 0,
          reasoning: 0,
        },
        cost: 0.25,
        messages: 1,
      }],
    };
    mockState.validateSubmission.mockReturnValue({
      valid: true,
      data: {
        meta: {
          generatedAt: "2026-07-14T00:00:00.000Z",
          version: "2.1.1",
          dateRange: { start: oldDay.date, end: trustedDay.date },
        },
        summary: {
          totalTokens: 500,
          totalCost: 1.25,
          totalDays: 2,
          activeDays: 2,
          averagePerDay: 250,
          maxCostInSingleDay: 1,
          clients: ["codex"],
          models: ["gpt-5.5"],
        },
        years: [{
          year: "2026",
          totalTokens: 500,
          totalCost: 1.25,
          range: { start: oldDay.date, end: trustedDay.date },
        }],
        contributions: [oldDay, trustedDay],
      },
      errors: [],
      warnings: [],
    });
    mockState.assessSubmissionTrust.mockReturnValue({
      trustState: "review_required",
      reasonCodes: ["historical_day_missing_timestamp"],
      rejectionReasonCodes: [],
      reviewDates: [oldDay.date],
      errors: [],
      warnings: ["old day needs review"],
    });
    mockState.clientContributionToBreakdownData.mockReturnValue({
      tokens: 100,
      cost: 0.25,
      input: 100,
      output: 0,
      cacheRead: 0,
      cacheWrite: 0,
      reasoning: 0,
      messages: 1,
    });
    mockState.recalculateDayTotals.mockReturnValue({
      tokens: 100,
      cost: 0.25,
      inputTokens: 100,
      outputTokens: 0,
    });

    const selectResults = [
      [{ pendingCount: 0, matchingHashCount: 0 }],
      [{ id: "submission-1" }],
      [],
      [{
        totalTokens: 100,
        totalCost: "0.2500",
        inputTokens: 100,
        outputTokens: 0,
        dateStart: trustedDay.date,
        dateEnd: trustedDay.date,
        activeDays: 1,
        totalActiveTimeMs: 0,
        rowCount: 1,
      }],
      [{
        sourceBreakdown: {
          codex: {
            cacheRead: 0,
            cacheWrite: 0,
            reasoning: 0,
            models: { "gpt-5.5": { tokens: 100 } },
          },
        },
      }],
    ];
    let insertCall = 0;
    let reviewInsertValues: Record<string, unknown> | undefined;
    let dailyInsertValues: Array<Record<string, unknown>> | undefined;
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
            values: vi.fn((values: Record<string, unknown>) => {
              reviewInsertValues = values;
              return builder;
            }),
            onConflictDoUpdate: vi.fn(() => builder),
            returning: vi.fn(() => Promise.resolve([{ id: "review-1" }])),
          };
          return builder;
        }
        if (insertCall === 2) {
          const builder = {
            values: vi.fn(() => builder),
            onConflictDoUpdate: vi.fn(() => builder),
            returning: vi.fn(() =>
              Promise.resolve([{ id: "submitted-device-1" }])
            ),
          };
          return builder;
        }
        return {
          values: vi.fn((values: Array<Record<string, unknown>>) => {
            dailyInsertValues = values;
            return Promise.resolve();
          }),
        };
      }),
      execute: vi.fn(() => Promise.resolve()),
      transaction: vi.fn(async (callback: (sp: typeof tx) => Promise<unknown>) =>
        callback(tx)
      ),
    };
    type MockTransaction = typeof tx;
    mockState.db.transaction.mockImplementation(
      async (callback: (transaction: MockTransaction) => Promise<unknown>) =>
        callback(tx)
    );

    const response = await POST(
      new Request("http://localhost:3000/api/submit", {
        method: "POST",
        headers: {
          Authorization: "Bearer tt_valid",
          "Content-Type": "application/json",
        },
        body: JSON.stringify({ mcpServers: ["github"] }),
      })
    );
    const body = await response.json();
    const reviewPayload = reviewInsertValues?.payload;

    expect(response.status).toBe(200);
    expect(tx.insert).toHaveBeenCalledTimes(3);
    expect(reviewInsertValues).toEqual(
      expect.objectContaining({
        totalTokens: 400,
        totalCost: "1.0000",
        dateStart: oldDay.date,
        dateEnd: oldDay.date,
        reasonCodes: ["historical_day_missing_timestamp"],
        schemaVersion: 1,
        competitiveWriteApplied: true,
      })
    );
    expect(reviewPayload).toBeDefined();
    if (
      !reviewPayload ||
      typeof reviewPayload !== "object" ||
      !("contributions" in reviewPayload) ||
      !("mcpServers" in reviewPayload)
    ) {
      throw new Error("Expected the queued review payload fields");
    }
    expect(reviewPayload.contributions).toEqual([
      expect.objectContaining({ date: oldDay.date }),
    ]);
    expect(reviewPayload.mcpServers).toEqual(["github"]);
    expect(dailyInsertValues).toEqual([
      expect.objectContaining({ date: trustedDay.date, tokens: 100 }),
    ]);
    expect(body).toEqual(
      expect.objectContaining({
        success: true,
        trustState: "review_required",
        submissionId: "submission-1",
        reviewId: "review-1",
        reviewMetrics: expect.objectContaining({
          totalTokens: 400,
          totalCost: 1,
          activeDays: 1,
        }),
        mode: "merge",
        competitiveWriteApplied: true,
        reasonCodes: ["historical_day_missing_timestamp"],
      })
    );
    expect(mockState.revalidateTag).toHaveBeenCalled();
    expect(mockState.revalidatePath).toHaveBeenCalledTimes(3);
    expect(mockState.revalidatePath).toHaveBeenNthCalledWith(1, "/");
    expect(mockState.revalidatePath).toHaveBeenNthCalledWith(2, "/api/leaderboard");
    expect(mockState.revalidatePath).toHaveBeenNthCalledWith(3, "/leaderboard");
  });
});
