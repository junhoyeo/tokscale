import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { users, moderationActions } from "@/lib/db/schema";

const mockState = vi.hoisted(() => {
  const authenticatePersonalToken = vi.fn();
  const validateSubmission = vi.fn();
  const generateSubmissionHash = vi.fn(() => "hash-123");
  const revalidateTag = vi.fn();
  const revalidatePath = vi.fn();
  const revalidateUsernamePaths = vi.fn();
  const revalidateUserGroupLeaderboards = vi.fn();
  const getLeaderboardData = vi.fn();

  const db = {
    transaction: vi.fn(),
  };

  return {
    authenticatePersonalToken,
    validateSubmission,
    generateSubmissionHash,
    revalidateTag,
    revalidatePath,
    revalidateUsernamePaths,
    revalidateUserGroupLeaderboards,
    getLeaderboardData,
    db,
    reset() {
      authenticatePersonalToken.mockReset();
      validateSubmission.mockReset();
      generateSubmissionHash.mockClear();
      revalidateTag.mockClear();
      revalidatePath.mockClear();
      revalidateUsernamePaths.mockReset();
      revalidateUserGroupLeaderboards.mockReset();
      getLeaderboardData.mockReset();
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
  apiTokens: { id: "apiTokens.id" },
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

vi.mock("@/lib/db/usernameLookup", () => ({
  normalizeUsernameCacheKey: (username: string) => username.toLowerCase(),
  revalidateUsernamePaths: mockState.revalidateUsernamePaths,
}));

vi.mock("@/lib/groups/cache", () => ({
  revalidateUserGroupLeaderboards: mockState.revalidateUserGroupLeaderboards,
}));

vi.mock("@/lib/leaderboard/getLeaderboard", () => ({
  getLeaderboardData: mockState.getLeaderboardData,
}));

type ModuleExports = typeof import("../../src/app/api/submit/route");
let POST: ModuleExports["POST"];

beforeEach(async () => {
  mockState.reset();
  delete process.env.ENABLE_SUBMISSION_AUTOHIDE;
  const routeModule = await import("../../src/app/api/submit/route");
  POST = routeModule.POST;
});

function makeAwaitableBuilder(result: unknown) {
  const builder = {
    from: vi.fn(() => builder),
    where: vi.fn(() => builder),
    for: vi.fn(() => builder),
    limit: vi.fn(() => builder),
    orderBy: vi.fn(() => builder),
    then: (resolve: (value: unknown) => unknown) => Promise.resolve(resolve(result)),
  };
  return builder;
}

interface MockTxOptions {
  userLeaderboardHidden?: boolean;
  usersUpdateReturnedRows?: Array<{ id: string }>;
  latestModAction?: { action: string; actorUsername: string | null } | null;
  aggregates?: {
    totalTokens: number;
    totalCost: string;
    inputTokens: number;
    outputTokens: number;
  };
  deviceTotals?: {
    totalActiveTimeMs: number;
    longestContinuousMs: number;
    maxConcurrentSessions: number;
    sessionCount: number;
  };
  allDaysSourceBreakdown?: Record<string, unknown>;
  allDaysRows?: unknown[];
}

function setupMockTx(options: MockTxOptions = {}) {
  const userHidden = options.userLeaderboardHidden ?? false;
  const usersReturned = options.usersUpdateReturnedRows ?? [{ id: "user-1" }];
  const latestAction = options.latestModAction !== undefined ? options.latestModAction : null;

  const aggregates = options.aggregates ?? {
    totalTokens: 10_000,
    totalCost: "0.0500",
    inputTokens: 5_000,
    outputTokens: 5_000,
  };

  const deviceTotals = options.deviceTotals ?? {
    totalActiveTimeMs: 60_000,
    longestContinuousMs: 30_000,
    maxConcurrentSessions: 1,
    sessionCount: 2,
  };

  const allDaysBreakdown = options.allDaysSourceBreakdown ?? {
    claude: {
      tokens: aggregates.totalTokens,
      cost: parseFloat(aggregates.totalCost),
      input: aggregates.inputTokens,
      output: aggregates.outputTokens,
      models: {
        "claude-3-5-sonnet-20241022": {
          tokens: aggregates.totalTokens,
          output: aggregates.outputTokens,
        },
      },
    },
  };

  const usersUpdates: unknown[] = [];
  const moderationActionInserts: unknown[] = [];
  const submissionsUpdates: unknown[] = [];

  const defaultAllDaysRows = [
    {
      date: "2026-04-30",
      tokens: aggregates.totalTokens,
      cost: aggregates.totalCost,
      inputTokens: aggregates.inputTokens,
      outputTokens: aggregates.outputTokens,
      sourceBreakdown: allDaysBreakdown,
    },
  ];

  const tx = {
    select: vi.fn((columns: Record<string, unknown>) => {
      const keys = Object.keys(columns);
      if (keys.includes("leaderboardHidden")) {
        return makeAwaitableBuilder([{ leaderboardHidden: userHidden }]);
      }
      if (keys.includes("action") && keys.includes("actorUsername")) {
        return makeAwaitableBuilder(latestAction ? [latestAction] : []);
      }
      if (keys.includes("totalTokens")) {
        return makeAwaitableBuilder([
          {
            ...aggregates,
            dateStart: "2026-04-30",
            dateEnd: "2026-04-30",
            activeDays: 1,
            rowCount: 1,
            costIsComplete: true,
          },
        ]);
      }
      if (keys.includes("totalActiveTimeMs")) {
        return makeAwaitableBuilder([deviceTotals]);
      }
      if (keys.includes("sourceBreakdown")) {
        return makeAwaitableBuilder(options.allDaysRows ?? defaultAllDaysRows);
      }
      // default: device / submission checks
      return makeAwaitableBuilder([{ id: "sub-1" }]);
    }),
    update: vi.fn((table: unknown) => {
      const builder = {
        set: vi.fn((values: unknown) => {
          if (table === users) {
            usersUpdates.push(values);
          } else {
            submissionsUpdates.push(values);
          }
          return builder;
        }),
        where: vi.fn(() => {
          if (table === users) {
            return {
              returning: vi.fn(() => Promise.resolve(usersReturned)),
              then: (resolve: (value: unknown) => unknown) => Promise.resolve(resolve(usersReturned)),
            };
          }
          return Promise.resolve();
        }),
      };
      return builder;
    }),
    insert: vi.fn((table: unknown) => {
      const builder = {
        values: vi.fn((values: unknown) => {
          if (table === moderationActions) {
            moderationActionInserts.push(values);
          }
          return builder;
        }),
        onConflictDoUpdate: vi.fn(() => builder),
        returning: vi.fn(() => Promise.resolve([{ id: "row-1" }])),
      };
      return builder;
    }),
    execute: vi.fn(() => Promise.resolve()),
  };

  mockState.db.transaction.mockImplementation(async (cb: (t: typeof tx) => Promise<unknown>) => cb(tx));

  return { tx, usersUpdates, moderationActionInserts, submissionsUpdates };
}

function createRequest(body: unknown) {
  return new Request("http://localhost:3000/api/submit", {
    method: "POST",
    headers: {
      Authorization: "Bearer tt_valid",
      "Content-Type": "application/json",
    },
    body: JSON.stringify(body),
  });
}

describe("POST /api/submit - ledger withholding", () => {
  beforeEach(() => {
    vi.stubEnv("ENABLE_SUBMISSION_AUTOHIDE", "1");
    vi.spyOn(console, "warn").mockImplementation(() => {});
    mockState.authenticatePersonalToken.mockResolvedValue({
      status: "valid", tokenId: "token-1", userId: "user-1", username: "alice",
      displayName: "Alice", avatarUrl: null, expiresAt: null,
    });
  });

  afterEach(() => {
    vi.unstubAllEnvs();
    vi.restoreAllMocks();
  });

  async function submit(options: MockTxOptions = {}) {
    const context = setupMockTx(options);
    mockState.validateSubmission.mockReturnValue({
      valid: true,
      data: {
        device: { id: "dev-1", name: "Laptop" },
        meta: { version: "4.20.0", dateRange: { start: "2026-04-30", end: "2026-04-30" } },
        summary: { clients: ["claude"], totalTokens: 10_000, totalCost: 0.05 },
        contributions: [{ date: "2026-04-30", clients: [] }],
      },
      errors: [], warnings: [],
    });
    const response = await POST(createRequest({}));
    expect(response.status).toBe(200);
    return context;
  }

  const excessiveOutput: MockTxOptions = {
    aggregates: { totalTokens: 20_000_000_000, totalCost: "1000", inputTokens: 0, outputTokens: 20_000_000_000 },
  };

  const excessivePerMessage: MockTxOptions = {
    aggregates: { totalTokens: 1_200_000_000, totalCost: "50", inputTokens: 1_200_000_000, outputTokens: 500 },
    allDaysSourceBreakdown: {
      claude: {
        tokens: 1_200_000_000,
        cost: 50,
        input: 1_200_000_000,
        output: 500,
        messages: 1,
        models: {
          "claude-3-5-sonnet": {
            tokens: 1_200_000_000,
            messages: 1,
          },
        },
      },
    },
  };

  it("keeps ordinary usage visible", async () => {
    const ctx = await submit();
    expect(ctx.usersUpdates).toHaveLength(0);
    expect(ctx.moderationActionInserts).toHaveLength(0);
  });

  it("atomically hides excessive ledger output and invalidates landing pages", async () => {
    const ctx = await submit(excessiveOutput);
    expect(ctx.usersUpdates).toEqual([expect.objectContaining({ leaderboardHidden: true })]);
    expect(ctx.moderationActionInserts).toEqual([expect.objectContaining({
      actorUsername: "system", action: "hide", reason: "Under investigation",
    })]);
    expect(mockState.revalidatePath).toHaveBeenCalledWith("/");
    expect(mockState.revalidatePath).toHaveBeenCalledWith("/leaderboard");
    expect(console.warn).toHaveBeenCalledWith(expect.stringContaining('"applied":true'));
  });

  it("atomically hides physically impossible single-message token volume (1+ billion tokens / 1 msg)", async () => {
    const ctx = await submit(excessivePerMessage);
    expect(ctx.usersUpdates).toEqual([expect.objectContaining({ leaderboardHidden: true })]);
    expect(ctx.moderationActionInserts).toEqual([expect.objectContaining({
      actorUsername: "system", action: "hide", reason: "Under investigation",
    })]);
    expect(mockState.revalidatePath).toHaveBeenCalledWith("/");
    expect(mockState.revalidatePath).toHaveBeenCalledWith("/leaderboard");
    expect(console.warn).toHaveBeenCalledWith(expect.stringContaining('"trigger":"per_message_token_limit"'));
    expect(console.warn).toHaveBeenCalledWith(expect.stringContaining('"applied":true'));
  });

  it("observes physically impossible single-message token volume without hiding in observe mode", async () => {
    vi.stubEnv("ENABLE_SUBMISSION_AUTOHIDE", "observe");
    const ctx = await submit(excessivePerMessage);
    expect(ctx.usersUpdates).toHaveLength(0);
    expect(ctx.moderationActionInserts).toHaveLength(0);
    expect(console.warn).toHaveBeenCalledWith(expect.stringContaining('"mode":"observe"'));
    expect(console.warn).toHaveBeenCalledWith(expect.stringContaining('"trigger":"per_message_token_limit"'));
    expect(console.warn).toHaveBeenCalledWith(expect.stringContaining('"shouldWithhold":true'));
    expect(console.warn).toHaveBeenCalledWith(expect.stringContaining('"applied":false'));
  });

  it.each([undefined, "observe", "unexpected"])("observes without changing accounts in mode %s", async (mode) => {
    vi.stubEnv("ENABLE_SUBMISSION_AUTOHIDE", mode);
    const ctx = await submit(excessiveOutput);
    expect(ctx.usersUpdates).toHaveLength(0);
    expect(ctx.moderationActionInserts).toHaveLength(0);
    expect(ctx.tx.select.mock.calls.some(([cols]) => "leaderboardHidden" in cols)).toBe(false);
    expect(mockState.revalidatePath).not.toHaveBeenCalled();
    expect(console.warn).toHaveBeenCalledWith(expect.stringContaining('"mode":"observe"'));
    expect(console.warn).toHaveBeenCalledWith(expect.stringContaining('"shouldWithhold":true'));
    expect(console.warn).toHaveBeenCalledWith(expect.stringContaining('"applied":false'));
  });

  it("logs clean observations as well as limit violations", async () => {
    vi.stubEnv("ENABLE_SUBMISSION_AUTOHIDE", undefined);
    await submit();
    expect(console.warn).toHaveBeenCalledWith(expect.stringContaining('"shouldWithhold":false'));
  });

  it("disables evaluations and their logs with the kill switch", async () => {
    vi.stubEnv("ENABLE_SUBMISSION_AUTOHIDE", "0");
    const ctx = await submit(excessiveOutput);
    expect(ctx.usersUpdates).toHaveLength(0);
    expect(ctx.moderationActionInserts).toHaveLength(0);
    expect(console.warn).not.toHaveBeenCalled();
  });

  it.each(["admin", "  AdMiN  "])("respects an administrator restore by %s", async (actorUsername) => {
    const ctx = await submit({ ...excessiveOutput, latestModAction: { action: "unhide", actorUsername } });
    expect(ctx.usersUpdates).toHaveLength(0);
    expect(ctx.moderationActionInserts).toHaveLength(0);
  });

  it.each(["system", null, " "])("does not infer an administrator override from actor %s", async (actorUsername) => {
    const ctx = await submit({ ...excessiveOutput, latestModAction: { action: "unhide", actorUsername } });
    expect(ctx.usersUpdates).toHaveLength(1);
    expect(ctx.moderationActionInserts).toHaveLength(1);
  });

  it("does not overwrite an existing hide or add another moderation action", async () => {
    const ctx = await submit({ ...excessiveOutput, userLeaderboardHidden: true });
    expect(ctx.usersUpdates).toHaveLength(0);
    expect(ctx.moderationActionInserts).toHaveLength(0);
  });

  it("does not log a state transition when a concurrent writer already changed it", async () => {
    const ctx = await submit({ ...excessiveOutput, usersUpdateReturnedRows: [] });
    expect(ctx.moderationActionInserts).toHaveLength(0);
    expect(mockState.revalidatePath).not.toHaveBeenCalled();
  });

  it("does not treat gap-merged continuous time as an impossible duration", async () => {
    const ctx = await submit({
      deviceTotals: { totalActiveTimeMs: 180_000, longestContinuousMs: 299_999, maxConcurrentSessions: 1, sessionCount: 2 },
    });
    expect(ctx.usersUpdates).toHaveLength(0);
  });

  it("does not automatically hide a high output/partial-active-time ratio", async () => {
    const ctx = await submit({
      aggregates: { totalTokens: 50_000_000, totalCost: "100", inputTokens: 0, outputTokens: 50_000_000 },
      deviceTotals: { totalActiveTimeMs: 10_000, longestContinuousMs: 10_000, maxConcurrentSessions: 1, sessionCount: 1 },
    });
    expect(ctx.usersUpdates).toHaveLength(0);
  });

  it("does not automatically hide high implied prices or configured model names", async () => {
    const ctx = await submit({
      aggregates: { totalTokens: 1_000_000, totalCost: "10000", inputTokens: 500_000, outputTokens: 500_000 },
      allDaysSourceBreakdown: { claude: { tokens: 1_000_000, cost: 10_000, input: 500_000, output: 500_000, modelId: "fake-model", models: {} } },
    });
    expect(ctx.usersUpdates).toHaveLength(0);
  });

  it("uses ledger scalars even when retained nested models exceed the limit", async () => {
    const ctx = await submit({
      allDaysSourceBreakdown: { claude: { tokens: 10_000, output: 5_000, models: { old: { tokens: 1_000_000_000_000_000, output: 20_000_000_000 } } } },
    });
    expect(ctx.usersUpdates).toHaveLength(0);
  });

  it("aggregates scalar daily output across devices", async () => {
    const ctx = await submit({
      allDaysRows: [
        { date: "2026-04-30", tokens: 6_000_000_000, outputTokens: 6_000_000_000, sourceBreakdown: {} },
        { date: "2026-04-30", tokens: 6_000_000_000, outputTokens: 6_000_000_000, sourceBreakdown: {} },
      ],
    });
    expect(ctx.usersUpdates).toHaveLength(1);
  });

  it("does not infer one daily volume from a legitimate multi-day total", async () => {
    const ctx = await submit({
      aggregates: { totalTokens: 900_000_000_000, totalCost: "1000", inputTokens: 900_000_000_000, outputTokens: 0 },
      allDaysRows: [
        { date: "2026-04-29", tokens: 450_000_000_000, outputTokens: 0, sourceBreakdown: {} },
        { date: "2026-04-30", tokens: 450_000_000_000, outputTokens: 0, sourceBreakdown: {} },
      ],
    });
    expect(ctx.usersUpdates).toHaveLength(0);
  });
});
