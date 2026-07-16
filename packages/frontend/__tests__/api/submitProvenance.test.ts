import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

// Pins a load-bearing safety property of the `tokscale import` draft
// (see crates/tokscale-cli/src/commands/import.rs and
// src/lib/validation/submission.ts): a validated submission-level
// `provenance: { origin: "backfill", ... }` tag is currently accepted by
// `validateSubmission` but is NOT written anywhere by this route — no
// `submissions`/`dailyBreakdown` write includes it, and it is not echoed
// back in the response. Persisting it (and actually segregating backfilled
// data in ranking) is deferred to the maintainer per
// https://github.com/junhoyeo/tokscale/issues/888. If this test starts
// failing because a write now carries `provenance`, that's a deliberate
// change to the deferred design, not a regression to silently fix.
const mockState = vi.hoisted(() => {
  const authenticatePersonalToken = vi.fn();
  const validateSubmission = vi.fn();
  const generateSubmissionHash = vi.fn(() => "submission-hash");
  const revalidateTag = vi.fn();
  const revalidateUsernamePaths = vi.fn();
  const revalidateUserGroupLeaderboards = vi.fn();
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

describe("POST /api/submit backfill provenance", () => {
  it("accepts a validated backfill provenance tag but does not persist it anywhere", async () => {
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
          id: "dev_backfill",
          name: "Backfill import",
        },
        meta: {
          version: "4.5.3",
          dateRange: { start: "2026-05-11", end: "2026-05-11" },
        },
        summary: {
          clients: ["codex"],
        },
        contributions: [
          {
            date: "2026-05-11",
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
                messages: 0,
              },
            ],
          },
        ],
        // This is the field under test: a validated submission-level
        // provenance tag from `tokscale import`.
        provenance: { origin: "backfill", importer: "clawdboard" },
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
      messages: 0,
    });
    mockState.recalculateDayTotals.mockReturnValue({
      tokens: 12,
      cost: 0.5,
      inputTokens: 7,
      outputTokens: 5,
    });
    mockState.mergeTimestampMs.mockImplementation(
      (_existing: unknown, incoming: unknown) => incoming
    );

    const selectResults = [
      [], // no existing submission
      [], // no existing device days
      [
        {
          totalTokens: 12,
          totalCost: "0.5000",
          inputTokens: 7,
          outputTokens: 5,
          dateStart: "2026-05-11",
          dateEnd: "2026-05-11",
          activeDays: 1,
          rowCount: 1,
        },
      ],
      [
        {
          sourceBreakdown: {
            codex: {
              cacheRead: 0,
              cacheWrite: 0,
              reasoning: 0,
              modelId: "gpt-5.5",
              models: { "gpt-5.5": { tokens: 12 } },
            },
          },
        },
      ],
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
      transaction: vi.fn(async (callback: (sp: typeof tx) => Promise<unknown>) =>
        callback(tx)
      ),
    };
    type MockTransaction = typeof tx;

    mockState.db.transaction.mockImplementation(
      async (callback: (tx: MockTransaction) => Promise<unknown>) => callback(tx)
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
          provenance: { origin: "backfill", importer: "clawdboard" },
        }),
      })
    );

    expect(response.status).toBe(200);

    // The submission-level `provenance` tag must not leak into any of the
    // rows this route writes: not the submitted-device row, not the
    // daily_breakdown insert, and not the submissions update. Persisting it
    // (and actually segregating backfilled data) is deliberately deferred —
    // see https://github.com/junhoyeo/tokscale/issues/888.
    expect(submittedDeviceValues).toEqual(
      expect.not.objectContaining({ provenance: expect.anything() })
    );
    expect(dailyInsertValues).toEqual([
      expect.not.objectContaining({ provenance: expect.anything() }),
    ]);
    expect(submissionUpdateValues).toEqual(
      expect.not.objectContaining({ provenance: expect.anything() })
    );

    // Nor is it echoed back to the caller.
    const body = await response.json();
    expect(body).not.toHaveProperty("provenance");
  });
});
