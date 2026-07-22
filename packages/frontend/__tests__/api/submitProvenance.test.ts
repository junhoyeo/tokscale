import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

// Pins the Phase 1 backfill-provenance persistence contract of POST
// /api/submit (see https://github.com/junhoyeo/tokscale/issues/888 and
// src/lib/validation/submission.ts):
//
//   - A validated submission-level `provenance: { origin: "backfill" }` tag
//     sets `hasBackfill: true` on the submissions row, on BOTH the initial
//     insert and the end-of-transaction update.
//   - Every per-client entry written into daily_breakdown.source_breakdown
//     for that submission carries `provenance.origin === "backfill"`, on both
//     the new-day INSERT path and the existing-day UPDATE path.
//   - The flag is sticky: a later non-backfill submit OMITS `hasBackfill`
//     from the submissions update entirely, so it can never reset the flag.
//   - `origin: "cli"` (or no provenance at all) never sets the flag.
//   - Provenance stays excluded from the idempotency hash — see the
//     hash-exclusion tests in __tests__/lib/submissionProvenance.test.ts.
const mockState = vi.hoisted(() => {
  const authenticatePersonalToken = vi.fn();
  const validateSubmission = vi.fn();
  const generateSubmissionHash = vi.fn(() => "submission-hash");
  const revalidateTag = vi.fn();
  const revalidateUsernamePaths = vi.fn();
  const revalidateUserGroupLeaderboards = vi.fn();
  const mergeClientBreakdownsWithRegressionGuard = vi.fn();
  const recalculateDayTotals = vi.fn();
  // Mirrors the real helper's contract, including the origin passthrough the
  // route relies on so re-derivation cannot drop the backfill tag.
  const deriveClientBreakdownProvenance = vi.fn(
    (breakdown: {
      messages?: number;
      models?: Record<string, unknown>;
      provenance?: { origin?: string };
    }) => ({
      schemaVersion: 1,
      messageCount: breakdown.messages ?? 0,
      modelCount: breakdown.models ? Object.keys(breakdown.models).length : 0,
      ...(breakdown.provenance?.origin
        ? { origin: breakdown.provenance.origin }
        : {}),
    })
  );
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

/** Recursively collect every string reachable from a value (cycle-safe). */
function collectStrings(node: unknown, out: string[], seen = new Set<object>()): void {
  if (typeof node === "string") {
    out.push(node);
    return;
  }
  if (!node || typeof node !== "object") return;
  if (seen.has(node as object)) return;
  seen.add(node as object);
  if (Array.isArray(node)) {
    for (const item of node) collectStrings(item, out, seen);
    return;
  }
  for (const value of Object.values(node as Record<string, unknown>)) {
    collectStrings(value, out, seen);
  }
}

function sqlArgsContain(executedSqlArgs: unknown[], needle: string): boolean {
  const strings: string[] = [];
  for (const arg of executedSqlArgs) collectStrings(arg, strings);
  return strings.some((s) => s.includes(needle));
}

const BACKFILL_TAG = '"origin":"backfill"';

function validSubmissionData(provenance?: { origin: string; importer?: string }) {
  return {
    device: {
      id: "dev_1",
      name: "Device one",
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
    ...(provenance ? { provenance } : {}),
  };
}

function primeDataMocks(provenance?: { origin: string; importer?: string }) {
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
  // Passthrough of the incoming per-client breakdown, mimicking the real
  // helper closely enough for tag propagation on the existing-day path.
  mockState.mergeClientBreakdownsWithRegressionGuard.mockImplementation(
    (_existing: unknown, incoming: Record<string, unknown>) => ({
      merged: incoming,
      warnings: [],
      foldPreservedClients: new Set<string>(),
    })
  );
}

function primeAuthMock() {
  mockState.authenticatePersonalToken.mockResolvedValue({
    status: "valid",
    tokenId: "token-1",
    userId: "user-1",
    username: "alice",
    displayName: "Alice",
    avatarUrl: null,
    expiresAt: null,
  });
}

const AGGREGATES_ROW = {
  totalTokens: 12,
  totalCost: "0.5000",
  inputTokens: 7,
  outputTokens: 5,
  dateStart: "2026-05-11",
  dateEnd: "2026-05-11",
  activeDays: 1,
  totalActiveTimeMs: 0,
  rowCount: 1,
};

const ALL_DAYS_ROW = {
  sourceBreakdown: {
    codex: {
      cacheRead: 0,
      cacheWrite: 0,
      reasoning: 0,
      modelId: "gpt-5.5",
      models: { "gpt-5.5": { tokens: 12 } },
    },
  },
};

interface MockTxCapture {
  submissionInsertValues: () => unknown;
  submissionUpdateValues: () => unknown;
  executedSqlArgs: unknown[];
  submissionsInsertCount: () => number;
}

/**
 * Builds a mock drizzle transaction.
 *
 * @param selectResults consumed in call order: (1) existing-submission
 *   lookup, (2) existing device days (FOR UPDATE; per-device since #910),
 *   then (3) aggregates + (4) all-days rows. The legacy-adoption re-fetch
 *   only fires when the device-days select is empty on a pre-existing
 *   submission, which none of these scenarios trigger.
 * @param existingSubmission when true, the submissions INSERT branch is
 *   never taken and the first insert call is the submitted-device upsert.
 */
function buildMockTx(selectResults: unknown[][], existingSubmission: boolean) {
  let insertCall = 0;
  let submissionsInsertCount = 0;
  let submissionInsertValues: unknown;
  let submissionUpdateValues: unknown;
  const executedSqlArgs: unknown[] = [];

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
      const isSubmissionsInsert = !existingSubmission && insertCall === 1;
      if (isSubmissionsInsert) {
        submissionsInsertCount += 1;
        const builder = {
          values: vi.fn((values: unknown) => {
            submissionInsertValues = values;
            return builder;
          }),
          returning: vi.fn(() => Promise.resolve([{ id: "submission-1" }])),
        };
        return builder;
      }

      // submitted-device upsert
      const builder = {
        values: vi.fn(() => builder),
        onConflictDoUpdate: vi.fn(() => builder),
        returning: vi.fn(() => Promise.resolve([{ id: "submitted-device-1" }])),
      };
      return builder;
    }),
    execute: vi.fn((sqlArg: unknown) => {
      executedSqlArgs.push(sqlArg);
      return Promise.resolve();
    }),
    transaction: vi.fn(async (callback: (sp: typeof tx) => Promise<unknown>) =>
      callback(tx)
    ),
  };

  mockState.db.transaction.mockImplementation(
    async (callback: (transaction: typeof tx) => Promise<unknown>) => callback(tx)
  );

  const capture: MockTxCapture = {
    submissionInsertValues: () => submissionInsertValues,
    submissionUpdateValues: () => submissionUpdateValues,
    executedSqlArgs,
    submissionsInsertCount: () => submissionsInsertCount,
  };
  return capture;
}

function submitRequest(provenance?: { origin: string; importer?: string }) {
  return new Request("http://localhost:3000/api/submit", {
    method: "POST",
    headers: {
      Authorization: "Bearer tt_valid",
      "Content-Type": "application/json",
    },
    body: JSON.stringify({
      meta: {},
      contributions: [],
      ...(provenance ? { provenance } : {}),
    }),
  });
}

describe("POST /api/submit backfill provenance persistence (phase 1)", () => {
  it("persists a backfill submission: hasBackfill on insert and update, per-client tag on the new-day INSERT path", async () => {
    primeAuthMock();
    primeDataMocks({ origin: "backfill", importer: "clawdboard" });

    const capture = buildMockTx(
      [
        [], // no existing submission -> insert path
        [], // no existing device days -> new-day INSERT path
        [AGGREGATES_ROW],
        [ALL_DAYS_ROW],
      ],
      false
    );

    const response = await POST(
      submitRequest({ origin: "backfill", importer: "clawdboard" })
    );
    expect(response.status).toBe(200);

    // Insert path: the brand-new submissions row is flagged from the start.
    expect(capture.submissionInsertValues()).toEqual(
      expect.objectContaining({ hasBackfill: true })
    );

    // Update path (STEP 3e recalculation): the flag is (re)asserted true.
    expect(capture.submissionUpdateValues()).toEqual(
      expect.objectContaining({ hasBackfill: true })
    );

    // The batched daily_breakdown INSERT carries the per-client backfill tag
    // inside source_breakdown.
    expect(sqlArgsContain(capture.executedSqlArgs, BACKFILL_TAG)).toBe(true);

    // The raw importer label is provenance metadata, not usage data — it is
    // not persisted anywhere in phase 1.
    expect(sqlArgsContain(capture.executedSqlArgs, "clawdboard")).toBe(false);

    // Not echoed back to the caller.
    const body = await response.json();
    expect(body).not.toHaveProperty("provenance");
  });

  it("persists the per-client backfill tag through the existing-day UPDATE path and keeps hasBackfill true", async () => {
    primeAuthMock();
    primeDataMocks({ origin: "backfill", importer: "clawdboard" });

    const capture = buildMockTx(
      [
        [{ id: "submission-existing" }], // existing submission -> no insert
        [
          {
            id: "day-1",
            date: "2026-05-11",
            timestampMs: null,
            activeTimeMs: null,
            sourceBreakdown: {},
          },
        ], // existing device day for the same date -> UPDATE path
        [AGGREGATES_ROW],
        [ALL_DAYS_ROW],
      ],
      true
    );

    const response = await POST(
      submitRequest({ origin: "backfill", importer: "clawdboard" })
    );
    expect(response.status).toBe(200);

    expect(capture.submissionsInsertCount()).toBe(0);
    expect(capture.submissionUpdateValues()).toEqual(
      expect.objectContaining({ hasBackfill: true })
    );

    // The raw `UPDATE daily_breakdown ... FROM (VALUES ...)` statement's
    // source_breakdown JSON carries the per-client backfill tag.
    expect(sqlArgsContain(capture.executedSqlArgs, BACKFILL_TAG)).toBe(true);
  });

  it("a submission without provenance inserts hasBackfill=false and never writes a backfill tag", async () => {
    primeAuthMock();
    primeDataMocks();

    const capture = buildMockTx(
      [[], [], [AGGREGATES_ROW], [ALL_DAYS_ROW]],
      false
    );

    const response = await POST(submitRequest());
    expect(response.status).toBe(200);

    expect(capture.submissionInsertValues()).toEqual(
      expect.objectContaining({ hasBackfill: false })
    );
    // The submissions update omits the key entirely rather than writing false.
    expect(capture.submissionUpdateValues()).toEqual(
      expect.not.objectContaining({ hasBackfill: expect.anything() })
    );
    expect(sqlArgsContain(capture.executedSqlArgs, BACKFILL_TAG)).toBe(false);
  });

  it("cli-origin provenance does not set the backfill flag", async () => {
    primeAuthMock();
    primeDataMocks({ origin: "cli" });

    const capture = buildMockTx(
      [[], [], [AGGREGATES_ROW], [ALL_DAYS_ROW]],
      false
    );

    const response = await POST(submitRequest({ origin: "cli" }));
    expect(response.status).toBe(200);

    expect(capture.submissionInsertValues()).toEqual(
      expect.objectContaining({ hasBackfill: false })
    );
    expect(capture.submissionUpdateValues()).toEqual(
      expect.not.objectContaining({ hasBackfill: expect.anything() })
    );
    expect(sqlArgsContain(capture.executedSqlArgs, BACKFILL_TAG)).toBe(false);
  });

  it("a second normal submit after a backfill does not clear the flag (update omits hasBackfill)", async () => {
    primeAuthMock();
    primeDataMocks(); // live CLI submit, no provenance

    // The user's submissions row already exists (and, in the database, is
    // already flagged hasBackfill=true from an earlier `tokscale import`).
    const capture = buildMockTx(
      [
        [{ id: "submission-existing" }],
        [
          {
            id: "day-1",
            date: "2026-05-11",
            timestampMs: null,
            activeTimeMs: null,
            sourceBreakdown: {},
          },
        ],
        [AGGREGATES_ROW],
        [ALL_DAYS_ROW],
      ],
      true
    );

    const response = await POST(submitRequest());
    expect(response.status).toBe(200);

    // The ONLY write that could clear the flag is the submissions update.
    // Sticky semantics require the key to be absent, not `false`.
    const updateValues = capture.submissionUpdateValues();
    expect(updateValues).toBeDefined();
    expect(Object.keys(updateValues as Record<string, unknown>)).not.toContain(
      "hasBackfill"
    );
  });
});
