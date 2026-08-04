import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

// REPRO of #960: rescanning the same local history under a different timezone
// permanently inflates stored usage.
//
// The CLI buckets each message into a LOCAL calendar day
// (`timestamp_to_date` uses `chrono::Local`), so a scan under Asia/Seoul and
// a rescan under UTC can split an unchanged midnight-crossing session across
// different days. The server merge only visits days present in the payload
// and never removes a stored day, so the day that legitimately emptied keeps
// its stale row while the new neighbour day is inserted -- the account is
// credited twice:
//
//   stored (Seoul scan):  2026-03-03 = 1000
//   rescan (UTC):         usage moves to 2026-03-02, 03-03 absent
//   result:               03-02 = 1000 (inserted) + 03-03 = 1000 (untouched)
//                          totalTokens = 2000, truth is 1000
//
// The regression guard cannot see this: it defends per-client decreases
// WITHIN a day, and the emptied day is not part of the payload at all.
//
// These tests characterize CURRENT submit behavior (the inflation is real and
// silent). The offline recovery described in docs/ratchet-inflation-recovery.md
// is intentionally out of this route's scope. The CLI-side fix in #1016
// (pinned `scanner.bucketTimezone`) stops new re-splits from pinned devices;
// the last test asserts the server-side consequence -- a same-day rescan
// merges flat instead of inflating.
//
// This is a route harness: it uses the real merge helpers and a stateful
// transaction double. The double records the route's daily_breakdown SQL
// writes, applies them to an in-memory table, and derives the later aggregate
// read from that table. Validation remains mocked only to keep this focused on
// the merge/write path; each test asserts the parsed request reaches that
// boundary unchanged.
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
    totalActiveTimeMs: "submissions.totalActiveTimeMs",
    longestContinuousMs: "submissions.longestContinuousMs",
    maxConcurrentSessions: "submissions.maxConcurrentSessions",
    sessionCount: "submissions.sessionCount",
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
    then: (resolve: (value: unknown) => unknown) =>
      Promise.resolve(resolve(result)),
  };
  return builder;
}

/** Recursively collect every string reachable from a value (cycle-safe). */
function collectStrings(
  node: unknown,
  out: string[],
  seen = new Set<object>(),
): void {
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

function storedClientEntry(tokens: number) {
  return {
    tokens,
    cost: tokens / 1000,
    input: tokens,
    output: 0,
    cacheRead: 0,
    cacheWrite: 0,
    reasoning: 0,
    messages: 5,
    models: {
      "test-model": {
        tokens,
        cost: tokens / 1000,
        input: tokens,
        output: 0,
        cacheRead: 0,
        cacheWrite: 0,
        reasoning: 0,
        messages: 5,
      },
    },
  };
}

function aggregatesRow(
  days: Array<{
    date: string;
    sourceBreakdown: Record<string, { tokens: number }>;
  }>,
) {
  const totalTokens = days.reduce(
    (total, day) =>
      total +
      Object.values(day.sourceBreakdown).reduce(
        (sum, client) => sum + client.tokens,
        0,
      ),
    0,
  );
  const dates = days.map((day) => day.date).sort();
  return {
    totalTokens,
    totalCost: (totalTokens / 1000).toFixed(4),
    inputTokens: totalTokens,
    outputTokens: 0,
    dateStart: dates[0] ?? null,
    dateEnd: dates[dates.length - 1] ?? null,
    activeDays: days.length,
    totalActiveTimeMs: 0,
    rowCount: days.length,
  };
}

type PersistedDay = {
  id: string;
  date: string;
  timestampMs: number | null;
  activeTimeMs: number | null;
  sourceBreakdown: Record<string, { tokens: number }>;
};

function buildTx(initialDays: PersistedDay[]) {
  const executedSqlArgs: unknown[] = [];
  const persistedDays = structuredClone(initialDays);
  let selectNumber = 0;

  function applyDailyBreakdownWrite(sqlArg: unknown): void {
    const strings: string[] = [];
    collectStrings(sqlArg, strings);
    const breakdownJson = strings.find(
      (value) => value.startsWith("{") && value.includes('"tokens"'),
    );
    if (!breakdownJson) return;

    const sourceBreakdown = JSON.parse(
      breakdownJson,
    ) as PersistedDay["sourceBreakdown"];
    if (
      strings.some((value) => value.includes("INSERT INTO daily_breakdown"))
    ) {
      const date = strings.find((value) => /^\d{4}-\d{2}-\d{2}$/.test(value));
      if (!date) throw new Error("daily breakdown INSERT did not bind a date");
      persistedDays.push({
        id: `inserted-${persistedDays.length + 1}`,
        date,
        timestampMs: null,
        activeTimeMs: null,
        sourceBreakdown,
      });
      return;
    }

    if (strings.some((value) => value.includes("UPDATE daily_breakdown"))) {
      const target = persistedDays.find((day) => strings.includes(day.id));
      if (!target)
        throw new Error("daily breakdown UPDATE did not bind a known row id");
      target.sourceBreakdown = sourceBreakdown;
    }
  }

  function nextSelectResult(): unknown[] {
    switch (selectNumber++) {
      case 0:
        return existingSubmissionRow();
      case 1:
        return persistedDays;
      case 2:
        return [aggregatesRow(persistedDays)];
      case 3:
        return [{}];
      case 4:
        return persistedDays.map(({ sourceBreakdown }) => ({
          sourceBreakdown,
        }));
      default:
        throw new Error(`unexpected SELECT #${selectNumber}`);
    }
  }

  const tx = {
    update: vi.fn(() => {
      const builder = {
        set: vi.fn(() => builder),
        where: vi.fn(() => Promise.resolve()),
      };
      return builder;
    }),
    select: vi.fn(() => makeAwaitableBuilder(nextSelectResult())),
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
      applyDailyBreakdownWrite(sqlArg);
      return Promise.resolve();
    }),
    transaction: vi.fn(async (callback: (sp: typeof tx) => Promise<unknown>) =>
      callback(tx),
    ),
  };
  mockState.db.transaction.mockImplementation(
    async (callback: (transaction: typeof tx) => Promise<unknown>) =>
      callback(tx),
  );
  return { tx, executedSqlArgs, persistedDays };
}

function submissionBody(
  contributions: Array<{
    date: string;
    client: string;
    tokens: number;
  }>,
) {
  const dates = contributions.map((c) => c.date).sort();
  return {
    device: { id: "dev_1", name: "Device one" },
    meta: {
      generatedAt: "2026-03-03T00:00:00Z",
      version: "4.10.0",
      dateRange: { start: dates[0], end: dates[dates.length - 1] },
    },
    summary: {
      clients: Array.from(new Set(contributions.map((c) => c.client))),
    },
    years: [],
    contributions: contributions.map((c) => ({
      date: c.date,
      clients: [
        {
          client: c.client,
          modelId: "test-model",
          tokens: {
            input: c.tokens,
            output: 0,
            cacheRead: 0,
            cacheWrite: 0,
            reasoning: 0,
          },
          cost: c.tokens / 1000,
          messages: 5,
        },
      ],
    })),
  };
}

function mockResubmit(body: ReturnType<typeof submissionBody>) {
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
    errors: [],
    warnings: [],
    data: body,
  });
}

function existingSubmissionRow() {
  return [
    {
      id: "submission-existing",
      totalActiveTimeMs: null,
      longestContinuousMs: null,
      maxConcurrentSessions: null,
      sessionCount: null,
    },
  ];
}

function post(body: object) {
  return POST(
    new Request("http://localhost:3000/api/submit", {
      method: "POST",
      headers: {
        Authorization: "Bearer tt_valid",
        "Content-Type": "application/json",
      },
      body: JSON.stringify(body),
    }),
  );
}

describe("POST /api/submit timezone re-split vs monotonic merge (#960)", () => {
  it("double-counts a day that moved across midnight when the rescan lands on the neighbour day", async () => {
    // Stored by the first scan (Asia/Seoul): the whole 1000-token session is
    // on 2026-03-03. The rescan from UTC moves it to 2026-03-02, so the
    // payload only mentions 03-02.
    const body = submissionBody([
      { date: "2026-03-02", client: "claude", tokens: 1000 },
    ]);
    mockResubmit(body);

    const storedSeoulDay = {
      id: "day-0303",
      date: "2026-03-03",
      timestampMs: null,
      activeTimeMs: null,
      sourceBreakdown: { claude: storedClientEntry(1000) },
    };

    const { tx, executedSqlArgs, persistedDays } = buildTx([storedSeoulDay]);
    void tx;

    const response = await post(body);
    expect(response.status).toBe(200);
    const json = await response.json();

    // The stored total is now 2000 for 1000 tokens of truth. The inflation is
    // SILENT: the emptied day is absent from the payload, so the regression
    // guard never fires and no warning is emitted.
    expect(json.mode).toBe("merge");
    expect(json.metrics.totalTokens).toBe(2000);
    expect(json.warnings).toBeUndefined();
    expect(mockState.validateSubmission).toHaveBeenCalledWith(body);

    // The moved day is inserted fresh (the source_breakdown JSON is passed as
    // a standalone parameter, the date as another).
    const strings: string[] = [];
    for (const arg of executedSqlArgs) collectStrings(arg, strings);
    const breakdownJsons = strings.filter(
      (s) => s.startsWith("{") && s.includes('"claude"'),
    );
    expect(breakdownJsons).toHaveLength(1);
    const inserted = JSON.parse(breakdownJsons[0]) as {
      claude: { tokens: number };
    };
    expect(inserted.claude.tokens).toBe(1000);
    expect(strings).toContain("2026-03-02");

    expect(persistedDays).toEqual([
      storedSeoulDay,
      expect.objectContaining({
        date: "2026-03-02",
        sourceBreakdown: expect.objectContaining({
          claude: expect.objectContaining({ tokens: 1000 }),
        }),
      }),
    ]);

    // ...and the stale day is never touched: no UPDATE ran at all.
    expect(strings.some((s) => s.includes("UPDATE daily_breakdown"))).toBe(
      false,
    );
  });

  it("lets the regression guard preserve the client that moved off a day", async () => {
    // Asia/Seoul scan: two clients on 2026-03-02 (500 each). The UTC rescan
    // moves codex's messages to 03-03, so the payload reports 03-02 with
    // claude only plus 03-03 with codex.
    const body = submissionBody([
      { date: "2026-03-02", client: "claude", tokens: 500 },
      { date: "2026-03-03", client: "codex", tokens: 500 },
    ]);
    mockResubmit(body);

    const storedSeoulDay = {
      id: "day-0302",
      date: "2026-03-02",
      timestampMs: null,
      activeTimeMs: null,
      sourceBreakdown: {
        claude: storedClientEntry(500),
        codex: storedClientEntry(500),
      },
    };

    const { tx, executedSqlArgs } = buildTx([storedSeoulDay]);
    void tx;

    const response = await post(body);
    expect(response.status).toBe(200);
    const json = await response.json();

    expect(json.metrics.totalTokens).toBe(1500);
    expect(
      json.warnings.some((w: string) => w.includes("Preserved codex")),
    ).toBe(true);
    expect(mockState.validateSubmission).toHaveBeenCalledWith(body);

    const strings: string[] = [];
    for (const arg of executedSqlArgs) collectStrings(arg, strings);

    // The UPDATE for 03-02 keeps the stale codex row next to the accepted
    // claude row: 500 + 500 on one day, plus codex's 500 inserted on 03-03.
    const updateJsons = strings.filter(
      (s) =>
        s.startsWith("{") && s.includes('"claude"') && s.includes('"codex"'),
    );
    expect(updateJsons).toHaveLength(1);
    const merged = JSON.parse(updateJsons[0]) as {
      claude: { tokens: number };
      codex: { tokens: number };
    };
    expect(merged.claude.tokens).toBe(500);
    expect(merged.codex.tokens).toBe(500);
    expect(strings).toContain("2026-03-03");

    const insertJsons = strings.filter(
      (s) =>
        s.startsWith("{") && s.includes('"codex"') && !s.includes('"claude"'),
    );
    expect(insertJsons).toHaveLength(1);
    const inserted = JSON.parse(insertJsons[0]) as {
      codex: { tokens: number };
    };
    expect(inserted.codex.tokens).toBe(500);
  });

  it("merges flat when the pinned timezone keeps the rescan on the same day (#1016)", async () => {
    // A pinned `scanner.bucketTimezone` (#1016) means the UTC rescan reports
    // the SAME day keys as the Seoul scan, so the payload still carries
    // 03-03. Equal tokens are accepted unchanged: no new row, no inflation.
    const body = submissionBody([
      { date: "2026-03-03", client: "claude", tokens: 1000 },
    ]);
    mockResubmit(body);

    const storedSeoulDay = {
      id: "day-0303",
      date: "2026-03-03",
      timestampMs: null,
      activeTimeMs: null,
      sourceBreakdown: { claude: storedClientEntry(1000) },
    };

    const { tx, executedSqlArgs } = buildTx([storedSeoulDay]);
    void tx;

    const response = await post(body);
    expect(response.status).toBe(200);
    const json = await response.json();

    expect(json.metrics.totalTokens).toBe(1000);
    expect(json.warnings).toBeUndefined();
    expect(mockState.validateSubmission).toHaveBeenCalledWith(body);

    const strings: string[] = [];
    for (const arg of executedSqlArgs) collectStrings(arg, strings);
    const updateJsons = strings.filter(
      (s) => s.startsWith("{") && s.includes('"claude"'),
    );
    expect(updateJsons).toHaveLength(1);
    const merged = JSON.parse(updateJsons[0]) as { claude: { tokens: number } };
    expect(merged.claude.tokens).toBe(1000);
    expect(strings.some((s) => s.includes("INSERT INTO daily_breakdown"))).toBe(
      false,
    );
  });
});
