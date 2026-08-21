import { beforeAll, beforeEach, afterEach, describe, expect, it, vi } from "vitest";

// Pins the PLACEMENT contract of the Phase 1 / Phase 1.5 ratchet census
// (docs/ratchet-inflation-recovery.md, issue #960):
//
//   - The high-water write runs AFTER the submit transaction commits, not
//     inside it. The submit path holds `.for('update')` on the submissions row
//     for the whole transaction, serializing a user's submits across every
//     device, so a defect in a write placed there would fail the user's
//     submission outright. "Nothing reads it" is no protection against that.
//   - Because the write is a GREATEST upsert it is idempotent, so a failure
//     after commit costs one deferred measurement — repaired by the next
//     submit — instead of a rejected submission. It must therefore never
//     propagate out of the handler.
//   - Phase 1.5 records a second derivation of the total and serves the
//     existing one unchanged.
//   - The write is killable by env flag without reverting a migration.
const mockState = vi.hoisted(() => {
  const authenticatePersonalToken = vi.fn();
  const validateSubmission = vi.fn();
  const generateSubmissionHash = vi.fn(() => "submission-hash");
  const revalidateTag = vi.fn();
  const revalidateUsernamePaths = vi.fn();
  const revalidateUserGroupLeaderboards = vi.fn();
  const mergeClientBreakdownsWithRegressionGuard = vi.fn();
  const recalculateDayTotals = vi.fn();
  const deriveClientBreakdownProvenance = vi.fn(() => ({
    schemaVersion: 1,
    messageCount: 0,
    modelCount: 1,
  }));
  const clientContributionToBreakdownData = vi.fn();
  const mergeTimestampMs = vi.fn();

  const db = {
    transaction: vi.fn(),
    execute: vi.fn(),
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
      db.execute.mockReset();
    },
  };
});

vi.mock("next/cache", () => ({ revalidateTag: mockState.revalidateTag }));

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
    ratchetCensusPending: "submissions.ratchetCensusPending",
    schemaVersion: "submissions.schemaVersion",
    hasBackfill: "submissions.hasBackfill",
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
    activeTimeMs: "dailyBreakdown.activeTimeMs",
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

vi.mock("@/lib/db/helpers", async (importOriginal) => ({
  // Spread the real module so a newly added export does not break every
  // test that mocks this file; only the named functions are stubbed.
  ...(await importOriginal<typeof import("@/lib/db/helpers")>()),
  mergeClientBreakdownsWithRegressionGuard:
    mockState.mergeClientBreakdownsWithRegressionGuard,
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

const flagName = "TOKSCALE_DEVICE_CLIENT_TOTALS_WRITE";

beforeEach(() => {
  mockState.reset();
  // The census now defaults OFF so it cannot add latency to `POST /api/submit`
  // by accident (see the flag's doc comment). These tests exercise the census
  // itself, so they opt in explicitly rather than depending on the production
  // default — which is what a test asserting placement should do anyway.
  process.env[flagName] = "1";
});

afterEach(() => {
  delete process.env[flagName];
  vi.restoreAllMocks();
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

const AGGREGATES_ROW = {
  totalTokens: 12,
  totalCost: "0.5000",
  inputTokens: 7,
  outputTokens: 5,
  dateStart: "2026-05-11",
  dateEnd: "2026-05-11",
  activeDays: 1,
  rowCount: 1,
};

const ALL_DAYS_ROW = {
  sourceBreakdown: {
    codex: { cacheRead: 0, cacheWrite: 0, reasoning: 0, models: { "gpt-5.5": { tokens: 12 } } },
  },
};

function validSubmissionData(provenance?: { origin: string }) {
  return {
    device: { id: "dev_1", name: "Device one" },
    meta: { version: "4.5.3", dateRange: { start: "2026-05-11", end: "2026-05-11" } },
    summary: { clients: ["codex"] },
    contributions: [
      {
        date: "2026-05-11",
        clients: [
          {
            client: "codex",
            modelId: "gpt-5.5",
            messages: 1,
            cost: 0.5,
            tokens: { input: 7, output: 5, cacheRead: 0, cacheWrite: 0, reasoning: 0 },
          },
        ],
      },
    ],
    ...(provenance ? { provenance } : {}),
  };
}

function primeMocks(provenance?: { origin: string }) {
  mockState.authenticatePersonalToken.mockResolvedValue({
    status: "valid",
    tokenId: "token-1",
    userId: "11111111-1111-4111-8111-111111111111",
    username: "alice",
    displayName: "Alice",
    avatarUrl: null,
    expiresAt: null,
  });
  mockState.validateSubmission.mockReturnValue({
    valid: true,
    data: validSubmissionData(provenance),
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
  mockState.mergeTimestampMs.mockImplementation((_e: unknown, i: unknown) => i);
  mockState.mergeClientBreakdownsWithRegressionGuard.mockImplementation(
    (_existing: unknown, incoming: Record<string, unknown>) => ({
      merged: incoming,
      warnings: [],
      foldPreservedClients: new Set<string>(),
    })
  );
}

interface TxTrace {
  /** Statements executed INSIDE the submit transaction. */
  inTransaction: unknown[];
  /** True once the transaction callback has returned (i.e. commit point). */
  committed: () => boolean;
}

function buildMockTx(): TxTrace {
  const inTransaction: unknown[] = [];
  let committed = false;
  const selectResults: unknown[][] = [
    [], // no existing submission -> insert path
    [], // no existing device days -> new-day INSERT path
    [AGGREGATES_ROW],
    [{ totalActiveTimeMs: 0, sessionCount: 0, longestContinuousMs: 0, maxConcurrentSessions: 0 }],
    [ALL_DAYS_ROW],
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
          returning: vi.fn(() => Promise.resolve([{ id: "submission-1" }])),
        };
        return builder;
      }
      const builder = {
        values: vi.fn(() => builder),
        onConflictDoUpdate: vi.fn(() => builder),
        returning: vi.fn(() =>
          Promise.resolve([{ id: "22222222-2222-4222-8222-222222222222" }])
        ),
      };
      return builder;
    }),
    execute: vi.fn((sqlArg: unknown) => {
      inTransaction.push(sqlArg);
      return Promise.resolve();
    }),
    transaction: vi.fn(async (cb: (sp: typeof tx) => Promise<unknown>) => cb(tx)),
  };

  mockState.db.transaction.mockImplementation(
    async (cb: (transaction: typeof tx) => Promise<unknown>) => {
      const value = await cb(tx);
      committed = true;
      return value;
    }
  );

  return { inTransaction, committed: () => committed };
}

function submitRequest() {
  return new Request("http://localhost:3000/api/submit", {
    method: "POST",
    headers: { Authorization: "Bearer tt_valid", "Content-Type": "application/json" },
    body: JSON.stringify({ meta: {}, contributions: [] }),
  });
}

function renderExecutedSql(calls: unknown[][]): string[] {
  return calls.map(([arg]) => JSON.stringify(arg));
}

describe("POST /api/submit ratchet census placement (phase 1 / 1.5)", () => {
  it("writes the high-water rows AFTER the transaction commits, never inside it", async () => {
    primeMocks();
    const trace = buildMockTx();
    let committedWhenCensusRan: boolean | null = null;
    mockState.db.execute.mockImplementation(async () => {
      if (committedWhenCensusRan === null) committedWhenCensusRan = trace.committed();
      return [
        { snapshotTokens: 12, snapshotCost: "0.5000", bucketCount: 1, tokens: 12, cost: "0.5000" },
      ];
    });

    const response = await POST(submitRequest());
    expect(response.status).toBe(200);

    // The census upsert reached the db outside the transaction...
    expect(mockState.db.execute).toHaveBeenCalled();
    expect(committedWhenCensusRan).toBe(true);
    const outside = renderExecutedSql(mockState.db.execute.mock.calls);
    expect(outside.some((s) => s.includes("submitted_device_client_totals"))).toBe(true);
    // ...and no statement touching the census table ran inside it.
    const inside = JSON.stringify(trace.inTransaction);
    expect(inside).not.toContain("submitted_device_client_totals");
  });

  it("registers durable replay work inside the transaction before the daily rows are visible", async () => {
    primeMocks();
    const trace = buildMockTx();
    mockState.db.execute.mockResolvedValue([]);

    const response = await POST(submitRequest());

    expect(response.status).toBe(200);
    expect(JSON.stringify(trace.inTransaction)).toContain("ratchet_census_work");
  });

  it("does not fail the submit when the post-commit census write throws", async () => {
    primeMocks();
    buildMockTx();
    const consoleError = vi.spyOn(console, "error").mockImplementation(() => {});
    mockState.db.execute.mockRejectedValue(new Error("bigint out of range"));

    const response = await POST(submitRequest());

    expect(response.status).toBe(200);
    const body = await response.json();
    expect(body.success).toBe(true);
    // The served metrics are the SUM(daily_breakdown) values, untouched.
    expect(body.metrics.totalTokens).toBe(12);
    expect(consoleError).toHaveBeenCalledWith(
      "Ratchet census write failed (submission unaffected):",
      expect.any(Error)
    );
  });

  it("serves the SUM(daily) total unchanged even when the high-water total disagrees", async () => {
    primeMocks();
    buildMockTx();
    const consoleLog = vi.spyOn(console, "log").mockImplementation(() => {});
    mockState.db.execute.mockResolvedValue([
      { snapshotTokens: 12, snapshotCost: "0.5000", bucketCount: 1, tokens: 3, cost: "0.1000" },
    ]);

    const response = await POST(submitRequest());
    const body = await response.json();

    expect(body.metrics.totalTokens).toBe(12);
    const censusLog = consoleLog.mock.calls
      .map((call) => String(call[0]))
      .find((line) => line.startsWith("ratchet-census "));
    expect(censusLog).toBeDefined();
    const record = JSON.parse(censusLog!.slice("ratchet-census ".length));
    expect(record).toMatchObject({
      servedTokens: 12,
      highwaterTokens: 3,
      tokenDelta: 9,
      highwaterStatus: "known",
    });
  });

  it("defers the A/B comparison when B commits daily rows before its high-water upsert", async () => {
    primeMocks();
    buildMockTx();
    const consoleLog = vi.spyOn(console, "log").mockImplementation(() => {});

    // Deterministic A/B timing: A has finished its upsert and reads the
    // census after B committed its daily rows plus pending ledger entry, but
    // before B starts its own deferred upsert. The census must not emit the
    // apparent 9-token gap as a stable divergence.
    mockState.db.execute.mockResolvedValue([
      { snapshotTokens: 12, snapshotCost: "0.5000", censusPending: 2, bucketCount: 1, tokens: 3, cost: "0.1000" },
    ]);

    const response = await POST(submitRequest());
    expect(response.status).toBe(200);

    const censusLog = consoleLog.mock.calls
      .map((call) => String(call[0]))
      .find((line) => line.startsWith("ratchet-census "));
    const record = JSON.parse(censusLog!.slice("ratchet-census ".length));
    expect(record).toMatchObject({
      racedConcurrentSubmit: true,
      censusStatus: "pending",
      highwaterTokens: 3,
      tokenDelta: null,
      tokenRatio: null,
    });
  });

  it("records UNKNOWN, not zero, while the user has no buckets yet", async () => {
    primeMocks();
    buildMockTx();
    const consoleLog = vi.spyOn(console, "log").mockImplementation(() => {});
    mockState.db.execute.mockResolvedValue([
      { snapshotTokens: 12, snapshotCost: "0.5000", bucketCount: 0, tokens: 0, cost: "0" },
    ]);

    await POST(submitRequest());

    const censusLog = consoleLog.mock.calls
      .map((call) => String(call[0]))
      .find((line) => line.startsWith("ratchet-census "));
    const record = JSON.parse(censusLog!.slice("ratchet-census ".length));
    expect(record.highwaterStatus).toBe("unknown");
    expect(record.highwaterTokens).toBeNull();
    expect(record.tokenDelta).toBeNull();
  });

  it("is killable by env flag without reverting the migration", async () => {
    process.env[flagName] = "0";
    primeMocks();
    buildMockTx();

    const response = await POST(submitRequest());

    expect(response.status).toBe(200);
    expect(mockState.db.execute).not.toHaveBeenCalled();
  });
});
