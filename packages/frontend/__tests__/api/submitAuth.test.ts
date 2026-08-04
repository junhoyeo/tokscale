import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

const mockState = vi.hoisted(() => {
  const authenticatePersonalToken = vi.fn();
  const validateSubmission = vi.fn();
  const generateSubmissionHash = vi.fn(() => "submission-hash");
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

  const db = {
    transaction: vi.fn(),
  };

  return {
    authenticatePersonalToken,
    validateSubmission,
    generateSubmissionHash,
    revalidateTag,
    revalidateUsernamePaths,
    revalidateUserGroupLeaderboards,
    mergeClientBreakdowns,
    mergeClientBreakdownsWithRegressionGuard,
    recalculateDayTotals,
    deriveClientBreakdownProvenance,
    clientContributionToBreakdownData,
    mergeTimestampMs,
    db,
    reset() {
      authenticatePersonalToken.mockReset();
      validateSubmission.mockReset();
      generateSubmissionHash.mockClear();
      revalidateTag.mockClear();
      revalidateUsernamePaths.mockReset();
      revalidateUserGroupLeaderboards.mockReset();
      mergeClientBreakdowns.mockReset();
      mergeClientBreakdownsWithRegressionGuard.mockReset();
      recalculateDayTotals.mockReset();
      deriveClientBreakdownProvenance.mockClear();
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

type ModuleExports = typeof import("../../src/app/api/submit/route");

let POST: ModuleExports["POST"];

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

// Drizzle's `sql` template composes nested SQL/StringChunk objects (and
// `sql.join` adds another nesting level for batched VALUES lists). Rather
// than assert on the exact nesting shape -- which shifts whenever the query
// is restructured -- flatten every chunk's raw values into one array so
// assertions only care that the expected params were embedded somewhere.
function flattenSqlChunks(node: unknown): unknown[] {
  if (node && typeof node === "object" && Array.isArray((node as { queryChunks?: unknown }).queryChunks)) {
    return (node as { queryChunks: unknown[] }).queryChunks.flatMap(flattenSqlChunks);
  }
  if (node && typeof node === "object" && Array.isArray((node as { value?: unknown }).value)) {
    return (node as { value: unknown[] }).value;
  }
  return [node];
}

// Phase 4a writes `daily_breakdown_reported` in the same transaction, so raw
// `tx.execute` call counts include that shadow upsert. Tests that pin the
// guarded daily_breakdown path filter it out.
function isDailyBreakdownReportedSql(node: unknown): boolean {
  return flattenSqlChunks(node).some(
    (chunk) =>
      typeof chunk === "string" && chunk.includes("daily_breakdown_reported")
  );
}

function dailyBreakdownExecuteArgs(tx: {
  execute: { mock: { calls: unknown[][] } };
}): unknown[] {
  return tx.execute.mock.calls
    .map((call) => call[0])
    .filter((arg) => !isDailyBreakdownReportedSql(arg));
}

function dailyBreakdownReportedExecuteArgs(tx: {
  execute: { mock: { calls: unknown[][] } };
}): unknown[] {
  return tx.execute.mock.calls
    .map((call) => call[0])
    .filter(isDailyBreakdownReportedSql);
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
          values: vi.fn(() => Promise.resolve()),
        };
      }),
      execute: vi.fn((..._args: unknown[]) => Promise.resolve()),
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
    expect(tx.insert).toHaveBeenCalledTimes(2);
    expect(tx.insert).toHaveBeenNthCalledWith(2, expect.objectContaining({
      id: "submittedDevices.id",
    }));
    expect(submittedDeviceValues).toEqual(expect.objectContaining({
      userId: "user-1",
      deviceKey: "dev_test",
      displayName: "Test device",
    }));
    expect(dailyBreakdownExecuteArgs(tx)).toHaveLength(1);
    const insertChunks = flattenSqlChunks(dailyBreakdownExecuteArgs(tx)[0]);
    expect(insertChunks).toEqual(
      expect.arrayContaining([
        expect.stringContaining("INSERT INTO daily_breakdown"),
        "submission-1",
        "submitted-device-1",
        "2026-04-30",
      ]),
    );
    // Device-scoped upsert: the daily_breakdown INSERT must target the
    // per-device unique key so independent devices own distinct rows. Naming
    // the old account-level (submission_id, date) key would collapse them.
    expect(insertChunks).toEqual(
      expect.arrayContaining([
        expect.stringContaining("ON CONFLICT (submission_id, submitted_device_id, date)"),
      ]),
    );
    expect(
      insertChunks.some(
        (chunk) =>
          typeof chunk === "string" &&
          /ON CONFLICT \(submission_id, date\)/.test(chunk),
      ),
    ).toBe(false);
    const reportedQueries = dailyBreakdownReportedExecuteArgs(tx);
    expect(reportedQueries).toHaveLength(1);
    expect(flattenSqlChunks(reportedQueries[0])).toEqual(
      expect.arrayContaining([
        expect.stringContaining("INSERT INTO daily_breakdown_reported"),
        "submitted-device-1",
        "2026-04-30",
      ]),
    );
    expect(submissionUpdateValues).toEqual(
      expect.objectContaining({
        mcpServers: ["github", "slack"],
      })
    );
    expect(mockState.revalidateTag).toHaveBeenNthCalledWith(1, "leaderboard", "max");
    expect(mockState.revalidateTag).toHaveBeenNthCalledWith(2, "user:alice", "max");
    expect(mockState.revalidateTag).toHaveBeenNthCalledWith(3, "user-rank", "max");
    expect(mockState.revalidateTag).toHaveBeenNthCalledWith(4, "user-rank:alice", "max");
    expect(mockState.revalidateUserGroupLeaderboards).toHaveBeenCalledWith("user-1");
    expect(mockState.revalidateUsernamePaths).toHaveBeenCalledWith("Alice");
  });

  it("replaces same-device daily rows without inserting duplicate dates", async () => {
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
      foldPreservedClients: new Set<string>(),
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
        const builder = {
          values: vi.fn(() => builder),
          onConflictDoUpdate: vi.fn(() => builder),
          returning: vi.fn(() => Promise.resolve([{ id: "submitted-device-1" }])),
        };
        return builder;
      }),
      execute: vi.fn((..._args: unknown[]) => Promise.resolve()),
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
    expect(tx.insert).toHaveBeenCalledWith(expect.objectContaining({
      id: "submittedDevices.id",
    }));
    expect(dailyBreakdownExecuteArgs(tx)).toHaveLength(1);
    // A same-device update keeps ownership implicit in the selected row and
    // must not rewrite submitted_device_id.
    expect(flattenSqlChunks(dailyBreakdownExecuteArgs(tx)[0])).not.toEqual(
      expect.arrayContaining(["submitted-device-1"]),
    );
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
      expect.any(Set),
      expect.any(Map)
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

  it("keeps same-client usage additive across devices on the same date", async () => {
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
    mockState.recalculateDayTotals.mockReturnValue({
      tokens: 15,
      cost: 0.75,
      inputTokens: 10,
      outputTokens: 5,
    });
    mockState.mergeTimestampMs.mockReturnValue(456);

    const selectResults = [
      [{ id: "submission-1" }],
      // No rows owned by dev_phone. A different device's row for this date
      // exists in the database but must not enter this device-scoped merge.
      [],
      // Legacy-adoption re-fetch remains empty because another modern device
      // already owns the existing row.
      [],
      [{
        totalTokens: 27,
        totalCost: "1.2500",
        inputTokens: 17,
        outputTokens: 10,
        dateStart: "2026-04-30",
        dateEnd: "2026-04-30",
        activeDays: 1,
        rowCount: 2,
      }],
      [
        { sourceBreakdown: existingBreakdown },
        { sourceBreakdown: mergedBreakdown },
      ],
    ];

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
        const builder = {
          values: vi.fn(() => builder),
          onConflictDoUpdate: vi.fn(() => builder),
          returning: vi.fn(() => Promise.resolve([{ id: "submitted-device-phone" }])),
        };
        return builder;
      }),
      execute: vi.fn((..._args: unknown[]) => Promise.resolve()),
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
    expect(dailyBreakdownExecuteArgs(tx)).toHaveLength(2);
    expect(flattenSqlChunks(dailyBreakdownExecuteArgs(tx)[1])).toEqual(
      expect.arrayContaining([
        expect.stringContaining("INSERT INTO daily_breakdown"),
        "submitted-device-phone",
        expect.stringContaining("ON CONFLICT (submission_id, submitted_device_id, date)"),
      ]),
    );
    expect(mockState.mergeClientBreakdownsWithRegressionGuard).not.toHaveBeenCalled();
    expect(await response.json()).toEqual(expect.objectContaining({
      success: true,
      metrics: expect.objectContaining({
        totalTokens: 27,
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
      foldPreservedClients: new Set<string>(),
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
      execute: vi.fn((..._args: unknown[]) => Promise.resolve()),
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
    expect(dailyBreakdownExecuteArgs(tx)).toHaveLength(1);
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
      expect.any(Set),
      expect.any(Map)
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

  it("sums per-device session metrics across devices instead of taking a max", async () => {
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
      }],
      // deviceTotals: this desktop contributed 4_000ms / 1 session and a second
      // machine previously contributed 6_000ms / 2 sessions. Session counts are
      // additive across devices (independent local sessions); the shape metrics
      // are a max because concurrency and streak length are per-machine.
      [{
        totalActiveTimeMs: 10_000,
        sessionCount: 3,
        longestContinuousMs: 6_000,
        maxConcurrentSessions: 2,
      }],
      [{ sourceBreakdown: insertedBreakdown }],
    ];

    const submissionUpdateSets: Array<Record<string, unknown>> = [];
    const selectFields: Array<Record<string, unknown>> = [];
    let insertCall = 0;
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
      select: vi.fn((fields?: Record<string, unknown>) => {
        if (fields) selectFields.push(fields);
        return makeAwaitableBuilder(selectResults.shift() ?? []);
      }),
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
          values: vi.fn(() => Promise.resolve()),
        };
        return builder;
      }),
      execute: vi.fn((..._args: unknown[]) => Promise.resolve()),
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
    expect(dailyBreakdownExecuteArgs(tx)).toHaveLength(2);
    expect(flattenSqlChunks(dailyBreakdownExecuteArgs(tx)[1])).toEqual(
      expect.arrayContaining([
        expect.stringContaining("INSERT INTO daily_breakdown"),
        "submission-1",
        "submitted-device-2",
        "2026-04-30",
        4_000,
      ]),
    );
    // 3, not 1: the submitting device reported sessionCount 1, so a
    // max-across-devices merge would report 1 and drop the other machine.
    expect(submissionUpdateSets.at(-1)).toEqual(expect.objectContaining({
      totalActiveTimeMs: 10_000,
      longestContinuousMs: 6_000,
      maxConcurrentSessions: 2,
      sessionCount: 3,
    }));

    // deviceTotals is a mocked row, so the assertion above cannot tell SUM from
    // MAX -- it only proves the route reads the aggregate instead of the
    // submitting device's own snapshot. Pin the aggregate functions directly:
    // counts are additive across machines, shape metrics are not.
    const deviceAggregate = selectFields.find(
      (fields) => !("id" in fields) && "sessionCount" in fields,
    );
    expect(deviceAggregate).toBeDefined();
    expect(flattenSqlChunks(deviceAggregate!.sessionCount)).toEqual(
      expect.arrayContaining([expect.stringContaining("SUM(")]),
    );
    expect(flattenSqlChunks(deviceAggregate!.totalActiveTimeMs)).toEqual(
      expect.arrayContaining([expect.stringContaining("SUM(")]),
    );
    expect(flattenSqlChunks(deviceAggregate!.maxConcurrentSessions)).toEqual(
      expect.arrayContaining([expect.stringContaining("MAX(")]),
    );
    expect(flattenSqlChunks(deviceAggregate!.longestContinuousMs)).toEqual(
      expect.arrayContaining([expect.stringContaining("MAX(")]),
    );

    // Only the SUM columns need the clamp: SUM() widens its input, so casting
    // back to the column type overflows once the per-device values total past
    // it, aborting the submit. Pin the bounds too -- a wrong constant is as
    // broken as a missing LEAST().
    const sessionCountSql = flattenSqlChunks(deviceAggregate!.sessionCount).join(" ");
    expect(sessionCountSql).toContain("LEAST(");
    expect(sessionCountSql).toContain("2147483647");
    const activeTimeSql = flattenSqlChunks(deviceAggregate!.totalActiveTimeMs).join(" ");
    expect(activeTimeSql).toContain("LEAST(");
    expect(activeTimeSql).toContain("9223372036854775807");

    // Same overflow shape on the daily-breakdown totals. These feed the
    // leaderboard, so an unclamped SUM turns one inflated day into a 500 on
    // every subsequent submit for that user.
    const tokenAggregate = selectFields.find((fields) => "totalTokens" in fields);
    expect(tokenAggregate).toBeDefined();
    for (const column of ["totalTokens", "inputTokens", "outputTokens"] as const) {
      const columnSql = flattenSqlChunks(tokenAggregate![column]).join(" ");
      expect(columnSql).toContain("LEAST(");
      expect(columnSql).toContain("9223372036854775807");
    }

    // The ON CONFLICT arm is unreachable while the per-user submissions row
    // lock holds, but it must not be a silent hole in the monotonic guard if
    // that ever changes (or if duplicate dates straddle an INSERT chunk).
    expect(flattenSqlChunks(dailyBreakdownExecuteArgs(tx)[1])).toEqual(
      expect.arrayContaining([
        expect.stringContaining("GREATEST(daily_breakdown.active_time_ms"),
      ]),
    );
  });

  it("preserves same-device active time and session metrics when local history shrinks", async () => {
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
      foldPreservedClients: new Set<string>(),
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
        totalActiveTimeMs: 13_000,
        longestContinuousMs: 9_000,
        maxConcurrentSessions: 4,
        sessionCount: 12,
      }],
      [{
        id: "daily-1",
        date: "2026-04-30",
        timestampMs: 123,
        activeTimeMs: 7_000,
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
      }],
      // deviceTotals deliberately LOWER than the stored submission values, the
      // shape of the migration transition: device rows start with NULL metrics,
      // so until every device submits again the SUM under-reports. The stored
      // value must act as a floor or the account total would drop.
      [{
        totalActiveTimeMs: 5_000,
        sessionCount: 1,
        longestContinuousMs: 5_000,
        maxConcurrentSessions: 1,
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
      execute: vi.fn((..._args: unknown[]) => Promise.resolve()),
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
    expect(dailyBreakdownExecuteArgs(tx)).toHaveLength(1);
    const updateChunks = flattenSqlChunks(dailyBreakdownExecuteArgs(tx)[0]);
    expect(updateChunks).toEqual(
      expect.arrayContaining([
        expect.stringContaining("UPDATE daily_breakdown"),
        "daily-1",
        456,
        7_000,
      ]),
    );
    // The VALUES tuple is (id, tokens, cost, input, output, timestamp_ms,
    // active_time_ms, source_breakdown). arrayContaining is order-blind, so it
    // alone cannot catch timestamp_ms and active_time_ms being transposed --
    // which would write an epoch millisecond into active_time_ms. Pin the
    // relative order of the two bound parameters.
    expect(updateChunks.indexOf(456)).toBeGreaterThan(updateChunks.indexOf("daily-1"));
    expect(updateChunks.indexOf(7_000)).toBeGreaterThan(updateChunks.indexOf(456));
    expect(submissionUpdateSets.at(-1)).toEqual(expect.objectContaining({
      totalActiveTimeMs: 13_000,
      longestContinuousMs: 9_000,
      maxConcurrentSessions: 4,
      sessionCount: 12,
    }));
    // The preservation is silent otherwise: the CLI prints these warnings, so
    // the user's only signal that their local scan came back short.
    const body = await response.json();
    expect(body.warnings).toEqual(
      expect.arrayContaining([
        expect.stringContaining("Preserved 7000ms active time"),
      ]),
    );
  });
  it("accepts a larger incoming active time and keeps the device upsert monotonic", async () => {
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
            timestampMs: 789,
            activeTimeMs: 9_000,
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
          totalActiveTimeMs: 9_000,
          longestContinuousMs: 9_000,
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
      foldPreservedClients: new Set<string>(),
    });
    mockState.recalculateDayTotals.mockReturnValue({
      tokens: 15,
      cost: 0.75,
      inputTokens: 10,
      outputTokens: 5,
    });
    mockState.mergeTimestampMs.mockReturnValue(789);

    const selectResults = [
      [{ id: "submission-1" }],
      [{
        id: "daily-1",
        date: "2026-04-30",
        timestampMs: 123,
        // Smaller than the incoming 9_000: growth must not be clamped.
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
      }],
      [{
        totalActiveTimeMs: 9_000,
        sessionCount: 1,
        longestContinuousMs: 9_000,
        maxConcurrentSessions: 1,
      }],
      [{ sourceBreakdown: mergedBreakdown }],
    ];

    const submissionUpdateSets: Array<Record<string, unknown>> = [];
    const deviceUpsertSets: Array<Record<string, unknown>> = [];
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
          onConflictDoUpdate: vi.fn((config: { set?: Record<string, unknown> }) => {
            if (config?.set) deviceUpsertSets.push(config.set);
            return builder;
          }),
          returning: vi.fn(() => Promise.resolve([{ id: "submitted-device-1" }])),
        };
        return builder;
      }),
      execute: vi.fn((..._args: unknown[]) => Promise.resolve()),
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
    // 9_000, not the stored 2_000: the monotonic guard preserves the LARGER
    // value, it does not freeze the row at whatever landed first.
    const updateChunks = flattenSqlChunks(dailyBreakdownExecuteArgs(tx)[0]);
    expect(updateChunks).toEqual(expect.arrayContaining([9_000]));
    expect(updateChunks).not.toEqual(expect.arrayContaining([2_000]));

    // Nothing was preserved, so the shrink warning must stay silent.
    const body = await response.json();
    expect(body.warnings ?? []).not.toEqual(
      expect.arrayContaining([expect.stringContaining("Preserved")]),
    );

    // The per-device high-water mark is enforced in SQL, so assert the upsert
    // actually carries GREATEST rather than a plain overwrite.
    const deviceSet = deviceUpsertSets.at(-1) ?? {};
    for (const column of [
      "totalActiveTimeMs",
      "longestContinuousMs",
      "maxConcurrentSessions",
      "sessionCount",
    ]) {
      expect(flattenSqlChunks(deviceSet[column])).toEqual(
        expect.arrayContaining([expect.stringContaining("GREATEST")]),
      );
    }
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
      foldPreservedClients: new Set<string>(),
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
          values: vi.fn(() => Promise.resolve()),
        };
        return builder;
      }),
      execute: vi.fn((..._args: unknown[]) => Promise.resolve()),
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
    expect(dailyBreakdownExecuteArgs(tx)).toHaveLength(2);
    expect(mockState.mergeClientBreakdownsWithRegressionGuard).toHaveBeenCalledWith(
      legacyBreakdown,
      incomingBreakdownWithProvenance,
      expect.any(Set),
      expect.any(Map)
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

  it("fails the request instead of double-counting when legacy adoption errors non-recoverably", async () => {
    // Regression: the legacy-adoption savepoint must only swallow a unique
    // violation (23505) from a concurrent submit. Any other failure (deadlock,
    // timeout, permission) leaves the legacy rows unclaimed; falling through
    // would insert the incoming device's overlapping history as a second row
    // and silently inflate totals. Such errors must propagate as a 500.
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

    mockState.clientContributionToBreakdownData.mockReturnValue({
      tokens: 15,
      cost: 0.75,
      input: 10,
      output: 5,
      cacheRead: 0,
      cacheWrite: 0,
      reasoning: 0,
      messages: 1,
    });
    mockState.mergeTimestampMs.mockReturnValue(123);

    const selectResults = [
      [{ id: "submission-1" }],
      // First fetchExistingDeviceDays() for dev_laptop: empty, so the route
      // enters the legacy-adoption branch.
      [],
    ];

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
        const builder = {
          values: vi.fn(() => builder),
          onConflictDoUpdate: vi.fn(() => builder),
          returning: vi.fn(() => Promise.resolve([{ id: "submitted-device-1" }])),
        };
        return builder;
      }),
      execute: vi.fn((..._args: unknown[]) => Promise.resolve()),
      // Savepoint fails with a non-unique error (deadlock, SQLSTATE 40P01).
      transaction: vi.fn(async () => {
        throw Object.assign(new Error("deadlock detected"), { code: "40P01" });
      }),
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

    expect(response.status).toBe(500);
    // The merge/write path must never run after a non-recoverable adoption error.
    expect(mockState.mergeClientBreakdownsWithRegressionGuard).not.toHaveBeenCalled();
    expect(tx.execute).not.toHaveBeenCalled();
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
          values: vi.fn(() => Promise.resolve()),
        };
        return builder;
      }),
      execute: vi.fn((..._args: unknown[]) => Promise.resolve()),
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
    expect(dailyBreakdownExecuteArgs(tx)).toHaveLength(2);
    expect(flattenSqlChunks(dailyBreakdownExecuteArgs(tx)[1])).toEqual(
      expect.arrayContaining([
        expect.stringContaining("INSERT INTO daily_breakdown"),
        "submission-1",
        "submitted-device-2",
        "2026-04-30",
        15,
        JSON.stringify(insertedBreakdown),
      ]),
    );
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
});
