import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

// REPRO + FIX VERIFICATION: interplay of the #886 kilocode->kilo alias fold
// with the #611/#910 token-regression guard on the existing-day merge path
// in POST /api/submit.
//
// Precondition (the exact state #886 set out to heal): a stored
// daily_breakdown row carries BOTH a stale "kilocode" key and a "kilo" key
// for the SAME underlying usage (double-counted, per the comment in
// route.ts). Before this fix, normalizeClientBreakdownAliases folded them by
// SUMMING into one "kilo" entry (100 + 100 = 200), and
// mergeClientBreakdownsWithRegressionGuard then compared the incoming
// full-day "kilo" contribution (100) against the folded sum (200). Because
// 100 < 200, the guard preserved the double-counted 200 forever and emitted
// a misleading parser-regression warning on every resubmit.
//
// The fix: normalizeClientBreakdownAliases now reports which canonical keys
// absorbed multiple raw source keys (foldedClients), and the merge guard
// lets a complete-day incoming submission for a folded client REPLACE the
// fold instead of defending it.
//
// This test uses the REAL @/lib/db/helpers implementations (not mocks) and
// asserts the healed value.
const mockState = vi.hoisted(() => {
  const authenticatePersonalToken = vi.fn();
  const validateSubmission = vi.fn();
  const generateSubmissionHash = vi.fn(() => "submission-hash");
  const revalidateTag = vi.fn();
  const revalidateUsernamePaths = vi.fn();
  const revalidateUserGroupLeaderboards = vi.fn();
  const db = { transaction: vi.fn() };
  return {
    authenticatePersonalToken,
    validateSubmission,
    generateSubmissionHash,
    revalidateTag,
    revalidateUsernamePaths,
    revalidateUserGroupLeaderboards,
    db,
    reset() {
      authenticatePersonalToken.mockReset();
      validateSubmission.mockReset();
      generateSubmissionHash.mockClear();
      revalidateTag.mockClear();
      revalidateUsernamePaths.mockReset();
      revalidateUserGroupLeaderboards.mockReset();
      db.transaction.mockReset();
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

// NOTE: @/lib/db/helpers is intentionally NOT mocked -- the real merge/fold
// logic is under test.

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

const CLIENT_ENTRY_100 = {
  tokens: 100,
  cost: 1,
  input: 100,
  output: 0,
  cacheRead: 0,
  cacheWrite: 0,
  reasoning: 0,
  messages: 5,
  models: {
    "test-model": {
      tokens: 100,
      cost: 1,
      input: 100,
      output: 0,
      cacheRead: 0,
      cacheWrite: 0,
      reasoning: 0,
      messages: 5,
    },
  },
};

const AGGREGATES_ROW = {
  totalTokens: 100,
  totalCost: "1.0000",
  inputTokens: 100,
  outputTokens: 0,
  dateStart: "2026-05-11",
  dateEnd: "2026-05-11",
  activeDays: 1,
  totalActiveTimeMs: 0,
  rowCount: 1,
};

function buildTx(selectResults: unknown[][]) {
  const executedSqlArgs: unknown[] = [];
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
  return { tx, executedSqlArgs };
}

function mockKiloResubmit() {
  mockState.authenticatePersonalToken.mockResolvedValue({
    status: "valid",
    tokenId: "token-1",
    userId: "user-1",
    username: "alice",
    displayName: "Alice",
    avatarUrl: null,
    expiresAt: null,
  });

  // Incoming: the new CLI reports the COMPLETE day for kilo -- 100 tokens.
  mockState.validateSubmission.mockReturnValue({
    valid: true,
    errors: [],
    warnings: [],
    data: {
      device: { id: "dev_1", name: "Device one" },
      meta: {
        generatedAt: "2026-05-11T00:00:00Z",
        version: "4.6.0",
        dateRange: { start: "2026-05-11", end: "2026-05-11" },
      },
      summary: { clients: ["kilo"] },
      years: [],
      contributions: [
        {
          date: "2026-05-11",
          clients: [
            {
              client: "kilo",
              modelId: "test-model",
              tokens: { input: 100, output: 0, cacheRead: 0, cacheWrite: 0, reasoning: 0 },
              cost: 1,
              messages: 5,
            },
          ],
        },
      ],
    },
  });
}

describe("POST /api/submit kilocode alias fold vs regression guard", () => {
  it("a full-day kilo resubmit heals a stored kilocode+kilo double-counted day back to the incoming total", async () => {
    mockKiloResubmit();

    // Stored day: the pre-#886 double-count state -- the SAME 100-token day
    // written once under the legacy "kilocode" key and once under "kilo".
    const existingDay = {
      id: "day-1",
      date: "2026-05-11",
      timestampMs: null,
      activeTimeMs: null,
      sourceBreakdown: {
        kilocode: structuredClone(CLIENT_ENTRY_100),
        kilo: structuredClone(CLIENT_ENTRY_100),
      },
    };

    const { tx, executedSqlArgs } = buildTx([
      [{ id: "submission-existing" }],
      [existingDay],
      [AGGREGATES_ROW],
      [{ sourceBreakdown: existingDay.sourceBreakdown }],
    ]);
    void tx;

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

    // Pull the merged source_breakdown JSON out of the batched raw UPDATE.
    const strings: string[] = [];
    for (const arg of executedSqlArgs) collectStrings(arg, strings);
    const breakdownJson = strings.find((s) => s.includes('"kilo"'));
    expect(breakdownJson).toBeDefined();
    const merged = JSON.parse(breakdownJson!) as Record<
      string,
      { tokens: number }
    >;

    // The stale legacy key must be gone either way.
    expect(merged).not.toHaveProperty("kilocode");

    // HEALED semantics (what #886's fold exists to produce): the complete
    // incoming kilo day replaces the double-counted fold. Before this fix
    // this was 200 -- the fold summed kilocode+kilo to 200, then the
    // regression guard saw incoming 100 < 200 and preserved the inflated
    // value forever, emitting a parser-regression warning every resubmit.
    expect(merged.kilo.tokens).toBe(100);

    const body = await response.json();
    const warnings: string[] = body.warnings ?? [];
    // No misleading parser-regression warning should be emitted for the
    // healed alias fold.
    expect(warnings.join(" ")).not.toMatch(/would reduce/i);
  });

  it("does NOT heal from a partial resubmit below the fold's largest component", async () => {
    mockKiloResubmit();
    // Override the incoming day with a PARTIAL parse: 40 tokens, below the
    // 100-token largest single contribution to the stored fold. Healing from
    // it would trade the inflated 200 for a value lower than any truthful
    // complete day can be -- new data loss, the exact case the regression
    // guard exists for -- so the fold must be preserved instead.
    mockState.validateSubmission.mockReturnValue({
      valid: true,
      errors: [],
      warnings: [],
      data: {
        device: { id: "dev_1", name: "Device one" },
        meta: {
          generatedAt: "2026-05-11T00:00:00Z",
          version: "4.6.0",
          dateRange: { start: "2026-05-11", end: "2026-05-11" },
        },
        summary: { clients: ["kilo"] },
        years: [],
        contributions: [
          {
            date: "2026-05-11",
            clients: [
              {
                client: "kilo",
                modelId: "test-model",
                tokens: { input: 40, output: 0, cacheRead: 0, cacheWrite: 0, reasoning: 0 },
                cost: 0.4,
                messages: 2,
              },
            ],
          },
        ],
      },
    });

    const existingDay = {
      id: "day-1",
      date: "2026-05-11",
      timestampMs: null,
      activeTimeMs: null,
      sourceBreakdown: {
        kilocode: structuredClone(CLIENT_ENTRY_100),
        kilo: structuredClone(CLIENT_ENTRY_100),
      },
    };

    const { tx, executedSqlArgs } = buildTx([
      [{ id: "submission-existing" }],
      [existingDay],
      [AGGREGATES_ROW],
      [{ sourceBreakdown: existingDay.sourceBreakdown }],
    ]);
    void tx;

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

    const strings: string[] = [];
    for (const arg of executedSqlArgs) collectStrings(arg, strings);
    const breakdownJson = strings.find((s) => s.includes('"kilo"'));
    expect(breakdownJson).toBeDefined();
    const merged = JSON.parse(breakdownJson!) as Record<
      string,
      { tokens: number }
    >;

    // The fold is preserved (still inflated in total) rather than
    // undercounted — and critically, the RAW alias keys survive the
    // writeback. Persisting the collapsed {kilo: 200} here would make the
    // heal window one-shot: the floor is derived from the raw keys, so the
    // next truthful complete-day resubmit could never heal the day again.
    expect(merged.kilocode.tokens).toBe(100);
    expect(merged.kilo.tokens).toBe(100);

    const body = await response.json();
    const warnings: string[] = body.warnings ?? [];
    expect(warnings.join(" ")).toMatch(/would reduce/i);
  });

  it("still heals after an intervening partial resubmit (heal window is not one-shot)", async () => {
    // Step 1 (simulated by the previous test's semantics): a below-floor
    // partial resubmit preserved the fold, writing the RAW keys back. Step 2
    // (this test): the stored day still carries both raw keys, so a later
    // truthful complete-day 100-token resubmit must still heal to 100.
    mockKiloResubmit();

    const existingDay = {
      id: "day-1",
      date: "2026-05-11",
      timestampMs: null,
      activeTimeMs: null,
      // Exactly what the partial-resubmit writeback now persists.
      sourceBreakdown: {
        kilocode: structuredClone(CLIENT_ENTRY_100),
        kilo: structuredClone(CLIENT_ENTRY_100),
      },
    };

    const { tx, executedSqlArgs } = buildTx([
      [{ id: "submission-existing" }],
      [existingDay],
      [AGGREGATES_ROW],
      [{ sourceBreakdown: existingDay.sourceBreakdown }],
    ]);
    void tx;

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

    const strings: string[] = [];
    for (const arg of executedSqlArgs) collectStrings(arg, strings);
    const breakdownJson = strings.find((s) => s.includes('"kilo"'));
    expect(breakdownJson).toBeDefined();
    const merged = JSON.parse(breakdownJson!) as Record<
      string,
      { tokens: number }
    >;

    expect(merged).not.toHaveProperty("kilocode");
    expect(merged.kilo.tokens).toBe(100);
  });

  it("still guards a rename-only kilocode fold (no kilo key present) against a token decrease", async () => {
    mockKiloResubmit();

    // Stored day: a PURE rename -- only the legacy "kilocode" key exists,
    // nothing to fold it with. This must NOT be treated as a suspect
    // double count; the normal regression guard should still defend it.
    const existingDay = {
      id: "day-1",
      date: "2026-05-11",
      timestampMs: null,
      activeTimeMs: null,
      sourceBreakdown: {
        kilocode: { ...structuredClone(CLIENT_ENTRY_100), tokens: 500, input: 500 },
      },
    };

    const { tx, executedSqlArgs } = buildTx([
      [{ id: "submission-existing" }],
      [existingDay],
      [{ ...AGGREGATES_ROW, totalTokens: 500 }],
      [{ sourceBreakdown: existingDay.sourceBreakdown }],
    ]);
    void tx;

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

    const strings: string[] = [];
    for (const arg of executedSqlArgs) collectStrings(arg, strings);
    const breakdownJson = strings.find((s) => s.includes('"kilo"'));
    expect(breakdownJson).toBeDefined();
    const merged = JSON.parse(breakdownJson!) as Record<
      string,
      { tokens: number }
    >;

    // A 100-token incoming resubmit against a 500-token rename-only existing
    // value is a genuine decrease -- the guard must preserve it, unchanged
    // from pre-fix behavior.
    expect(merged.kilo.tokens).toBe(500);

    const body = await response.json();
    const warnings: string[] = body.warnings ?? [];
    expect(warnings.join(" ")).toMatch(/would reduce/i);
  });
});

// The fold builds its normalized view by copying each client entry, then sums
// into that copy in place. When the copy was shallow, the nested model objects
// were still the STORED ones, so folding a day mutated the raw breakdown that
// the fold-preservation writeback persists moments later -- and every later
// submit folded the already-folded values into themselves again. Client-level
// scalars were spread-copied and so stayed correct, which is why this only
// ever surfaced in per-model views.
type PersistedBreakdown = Record<
  string,
  { tokens: number; models: Record<string, { tokens: number }> }
>;

describe("POST /api/submit alias fold leaves the stored raw breakdown untouched", () => {
  /** 40 tokens is below the fold's 100-token largest component, so the guard
   * preserves the fold and the writeback restores BOTH raw alias keys -- the
   * path that persists whatever the fold did to the raw model objects. */
  function mockPartialKiloResubmit() {
    mockKiloResubmit();
    mockState.validateSubmission.mockReturnValue({
      valid: true,
      errors: [],
      warnings: [],
      data: {
        device: { id: "dev_1", name: "Device one" },
        meta: {
          generatedAt: "2026-05-11T00:00:00Z",
          version: "4.6.0",
          dateRange: { start: "2026-05-11", end: "2026-05-11" },
        },
        summary: { clients: ["kilo"] },
        years: [],
        contributions: [
          {
            date: "2026-05-11",
            clients: [
              {
                client: "kilo",
                modelId: "test-model",
                tokens: { input: 40, output: 0, cacheRead: 0, cacheWrite: 0, reasoning: 0 },
                cost: 0.4,
                messages: 2,
              },
            ],
          },
        ],
      },
    });
  }

  /** One fold-PRESERVING submit (incoming is below the heal floor, so the raw
   * alias keys are written back verbatim). Returns what was persisted. */
  async function submitBelowHealFloor(
    storedBreakdown: Record<string, unknown>
  ): Promise<PersistedBreakdown> {
    const existingDay = {
      id: "day-1",
      date: "2026-05-11",
      timestampMs: null,
      activeTimeMs: null,
      sourceBreakdown: storedBreakdown,
    };

    const { executedSqlArgs } = buildTx([
      [{ id: "submission-existing" }],
      [existingDay],
      [AGGREGATES_ROW],
      [{ sourceBreakdown: existingDay.sourceBreakdown }],
    ]);

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

    const strings: string[] = [];
    for (const arg of executedSqlArgs) collectStrings(arg, strings);
    const breakdownJson = strings.find(
      (s) => s.includes('"kilocode"') && s.includes('"models"')
    );
    expect(breakdownJson).toBeDefined();
    return JSON.parse(breakdownJson!) as PersistedBreakdown;
  }

  it("does not compound the raw alias keys' nested model values across repeated submits", async () => {
    mockPartialKiloResubmit();

    const storedBefore = {
      kilocode: structuredClone(CLIENT_ENTRY_100),
      kilo: structuredClone(CLIENT_ENTRY_100),
    };

    const afterFirst = await submitBelowHealFloor(storedBefore);

    // Each raw key still carries only ITS OWN 100 tokens at the model level.
    // Pre-fix this was 200: the fold summed kilo's models into the objects
    // kilocode's entry still pointed at.
    expect(afterFirst.kilocode.models["test-model"].tokens).toBe(100);
    expect(afterFirst.kilo.models["test-model"].tokens).toBe(100);
    // The model total must keep agreeing with the client total it decomposes.
    expect(afterFirst.kilocode.models["test-model"].tokens).toBe(
      afterFirst.kilocode.tokens
    );

    // Feed the persisted row back in, exactly as the next submit would read
    // it. Pre-fix this second pass produced 300, then 400, without bound.
    const afterSecond = await submitBelowHealFloor(afterFirst);

    expect(afterSecond.kilocode.models["test-model"].tokens).toBe(100);
    expect(afterSecond.kilo.models["test-model"].tokens).toBe(100);
    expect(afterSecond).toEqual(afterFirst);
  });

  it("leaves the caller's stored breakdown object unmutated by the fold", async () => {
    mockPartialKiloResubmit();

    const storedBreakdown = {
      kilocode: structuredClone(CLIENT_ENTRY_100),
      kilo: structuredClone(CLIENT_ENTRY_100),
    };
    const pristine = structuredClone(storedBreakdown);

    await submitBelowHealFloor(storedBreakdown);

    // The row object handed to the route is the parsed source_breakdown JSON;
    // the merge must treat it as read-only input, not scratch space.
    expect(storedBreakdown).toEqual(pristine);
  });
});
