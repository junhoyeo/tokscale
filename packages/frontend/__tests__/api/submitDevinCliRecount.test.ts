import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

import { SUPPORTED_VERSIONED_PARSERS } from "../../src/lib/db/parserHighWater";
import type { ClientBreakdownData } from "../../src/lib/db/helpers";

// End-to-end regression for the Devin CLI recount generation.
//
// Devin CLI's parser generation 1 deduplicated `message_nodes` rows by row id
// and counted each persisted copy of the same API request; generation 2
// deduplicates by `metadata.request_id`, so a corrected rescan reports the
// same history strictly LOWER. The per-day merge guard defends each stored
// (day, client) cell against a decrease, so on its own the inflated cells are
// locked in forever.
//
// The recounting-generation plan (`RECOUNTING_GENERATION_CLIENTS`) gets one
// rewrite at the transition: covered stored cells adopt the snapshot's values
// outright, uncovered stored days are preserved, and the ledger returns to
// the monotonic incremental path for every submit after.
//
// This is a route harness in the style of submitAntigravityCliHighWater: it
// runs the real merge/high-water code against a stateful transaction double
// that records the route's daily_breakdown writes, replays them into an
// in-memory table, and derives the later aggregate read from that table.
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
    parserVersions: "submittedDevices.parserVersions",
    parserStates: "submittedDevices.parserStates",
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

/** Recursively collect every string reachable from a value, in bind order. */
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

type StoredBreakdown = Record<string, ClientBreakdownData>;

type PersistedDay = {
  id: string;
  date: string;
  timestampMs: number | null;
  activeTimeMs: number | null;
  sourceBreakdown: StoredBreakdown;
};

type DeviceRow = {
  id: string;
  parserVersions: Record<string, number>;
  parserStates: Record<string, unknown>;
};

type Store = {
  days: PersistedDay[];
  device: DeviceRow;
  inserted: number;
};

function newStore(): Store {
  return {
    days: [],
    device: { id: "submitted-device-1", parserVersions: {}, parserStates: {} },
    inserted: 0,
  };
}

function devinCell(
  tokens: number,
  cost: number,
  messages = 1,
  modelId = "swe-2-high",
): ClientBreakdownData {
  return {
    tokens,
    cost,
    input: tokens,
    output: 0,
    cacheRead: 0,
    cacheWrite: 0,
    reasoning: 0,
    messages,
    models: {
      [modelId]: {
        tokens,
        cost,
        input: tokens,
        output: 0,
        cacheRead: 0,
        cacheWrite: 0,
        reasoning: 0,
        messages,
      },
    },
  };
}

function seedDay(store: Store, date: string, breakdown: StoredBreakdown) {
  store.days.push({
    id: `stored-${date}`,
    date,
    timestampMs: null,
    activeTimeMs: null,
    sourceBreakdown: breakdown,
  });
}

function storedTokens(store: Store): number {
  return store.days.reduce(
    (total, day) =>
      total +
      Object.values(day.sourceBreakdown).reduce(
        (sum, client) => sum + client.tokens,
        0,
      ),
    0,
  );
}

function storedCost(store: Store): number {
  return store.days.reduce(
    (total, day) =>
      total +
      Object.values(day.sourceBreakdown).reduce(
        (sum, client) => sum + (client.cost ?? 0),
        0,
      ),
    0,
  );
}

function storedClientTokens(store: Store, date: string, client: string) {
  return store.days.find((day) => day.date === date)?.sourceBreakdown[client]
    ?.tokens;
}

function aggregatesRow(store: Store) {
  const totalTokens = storedTokens(store);
  const dates = store.days
    .filter((day) =>
      Object.values(day.sourceBreakdown).some((client) => client.tokens > 0),
    )
    .map((day) => day.date)
    .sort();
  return {
    totalTokens,
    totalCost: storedCost(store).toFixed(4),
    inputTokens: totalTokens,
    outputTokens: 0,
    dateStart: dates[0] ?? null,
    dateEnd: dates[dates.length - 1] ?? null,
    activeDays: dates.length,
    rowCount: store.days.length,
    totalActiveTimeMs: 0,
  };
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

/**
 * Dispatch on the requested column shape rather than on call order: the route
 * re-reads the device's days after the legacy-adoption UPDATE, so the number
 * of SELECTs differs between a first and a later submit.
 */
function selectResult(columns: Record<string, unknown>, store: Store) {
  const keys = new Set(Object.keys(columns));
  if (keys.has("date") && keys.has("sourceBreakdown")) return store.days;
  if (keys.size === 1 && keys.has("sourceBreakdown")) {
    return store.days.map(({ sourceBreakdown }) => ({ sourceBreakdown }));
  }
  if (keys.has("totalTokens")) return [aggregatesRow(store)];
  if (keys.has("id") && keys.has("sessionCount")) return existingSubmissionRow();
  if (keys.has("sessionCount") && keys.has("totalActiveTimeMs")) return [{}];
  throw new Error(`unexpected SELECT shape: ${[...keys].join(",")}`);
}

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

function isDailyBreakdownInsert(text: string): boolean {
  // Must not match `INSERT INTO daily_breakdown_reported`.
  return /INSERT INTO daily_breakdown\b(?!_)/.test(text);
}

function installTx(store: Store) {
  function applyDailyBreakdownWrite(sqlArg: unknown): void {
    const strings: string[] = [];
    collectStrings(sqlArg, strings);
    const text = strings.join("\n");
    if (text.includes("DELETE FROM daily_breakdown")) {
      store.days = store.days.filter((day) => !strings.includes(day.id));
      return;
    }
    const insert = isDailyBreakdownInsert(text);
    // The batch row update; NOT the legacy-adoption `UPDATE ... AS db`, which
    // only re-stamps submitted_device_id and binds no breakdown JSON.
    const update = text.includes("UPDATE daily_breakdown AS d SET");
    if (!insert && !update) return;

    // A chunked statement carries many rows. Within a row clause the date
    // (INSERT) or the row id (UPDATE) is bound before that row's breakdown
    // JSON, so the most recent one seen owns the JSON that follows.
    let date: string | null = null;
    let rowId: string | null = null;
    for (const value of strings) {
      if (/^\d{4}-\d{2}-\d{2}$/.test(value)) {
        date = value;
        continue;
      }
      if (store.days.some((day) => day.id === value)) {
        rowId = value;
        continue;
      }
      if (!value.startsWith("{") || !value.includes('"tokens"')) continue;
      const sourceBreakdown = JSON.parse(value) as StoredBreakdown;
      if (insert) {
        if (!date) throw new Error("daily breakdown INSERT did not bind a date");
        const existing = store.days.find((day) => day.date === date);
        if (existing) {
          // Mirrors ON CONFLICT (submission_id, submitted_device_id, date).
          existing.sourceBreakdown = sourceBreakdown;
        } else {
          store.days.push({
            id: `inserted-${++store.inserted}`,
            date,
            timestampMs: null,
            activeTimeMs: null,
            sourceBreakdown,
          });
        }
        continue;
      }
      if (!rowId) throw new Error("daily breakdown UPDATE did not bind a row id");
      const target = store.days.find((day) => day.id === rowId);
      if (!target) throw new Error(`UPDATE bound unknown row id ${rowId}`);
      target.sourceBreakdown = sourceBreakdown;
    }
  }

  const tx = {
    update: vi.fn(() => {
      const builder = {
        set: vi.fn((payload: Record<string, unknown>) => {
          if (payload && "parserStates" in payload) {
            store.device.parserVersions = payload.parserVersions as Record<
              string,
              number
            >;
            store.device.parserStates = payload.parserStates as Record<
              string,
              unknown
            >;
          }
          return builder;
        }),
        where: vi.fn(() => Promise.resolve()),
      };
      return builder;
    }),
    select: vi.fn((columns: Record<string, unknown>) =>
      makeAwaitableBuilder(selectResult(columns, store)),
    ),
    insert: vi.fn(() => {
      const builder = {
        values: vi.fn(() => builder),
        onConflictDoUpdate: vi.fn(() => builder),
        returning: vi.fn(() =>
          Promise.resolve([
            {
              id: store.device.id,
              parserVersions: store.device.parserVersions,
              parserStates: store.device.parserStates,
            },
          ]),
        ),
      };
      return builder;
    }),
    execute: vi.fn((sqlArg: unknown) => {
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
}

function submissionBody(
  days: Array<{ date: string; tokens: number; cost?: number }>,
  options: { version?: number; fullHistory?: boolean; costIsComplete?: boolean } = {},
) {
  const version = options.version ?? SUPPORTED_VERSIONED_PARSERS["devin-cli"];
  const dates = days.map((day) => day.date).sort();
  return {
    device: { id: "dev_1", name: "Device one" },
    meta: {
      generatedAt: "2026-10-09T00:00:00Z",
      version: "4.18.0",
      dateRange: { start: dates[0], end: dates[dates.length - 1] },
    },
    scanScope: {
      parserVersions: { "devin-cli": version },
      fullHistory: options.fullHistory ?? true,
    },
    summary: { clients: ["devin-cli"] },
    years: [],
    contributions: days.map((day) => ({
      date: day.date,
      totals: {
        tokens: day.tokens,
        cost: day.cost ?? day.tokens / 1000,
        messages: 1,
        costIsComplete: options.costIsComplete ?? true,
      },
      clients: [
        {
          client: "devin-cli",
          modelId: "swe-2-high",
          tokens: {
            input: day.tokens,
            output: 0,
            cacheRead: 0,
            cacheWrite: 0,
            reasoning: 0,
          },
          cost: day.cost ?? day.tokens / 1000,
          messages: 1,
        },
      ],
    })),
  };
}

function mockSubmit(body: ReturnType<typeof submissionBody>) {
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

async function post(body: object) {
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

// What the inflated generation-1 submissions left in the device: each day
// counts duplicate `message_nodes` rows, roughly 2.2x the real usage. A third
// stored day has no local data left, so no corrected snapshot reports it.
const INFLATED_DAYS = [
  { date: "2026-10-06", tokens: 88_000_000, cost: 88 },
  { date: "2026-10-07", tokens: 514_000_000, cost: 514 },
  { date: "2026-10-08", tokens: 1_000_000_000, cost: 1000 },
];
const CORRECTED_SCAN = [
  { date: "2026-10-07", tokens: 234_000_000, cost: 234 },
  { date: "2026-10-08", tokens: 455_000_000, cost: 455 },
];

function seedInflatedDays(store: Store) {
  for (const day of INFLATED_DAYS) {
    seedDay(store, day.date, { "devin-cli": devinCell(day.tokens, day.cost) });
  }
}

describe("POST /api/submit devin-cli recount", () => {
  it("registers devin-cli at the generation the deduplicating CLI declares", () => {
    // Generation 2 is the request_id recount. Registering 1 would recount on
    // every still-inflating submission; registering above 2 freezes the only
    // CLI that can heal the stored rows.
    expect(SUPPORTED_VERSIONED_PARSERS["devin-cli"]).toBe(2);
  });

  it("lowers covered stored cells to the recounted values and keeps uncovered days", async () => {
    const store = newStore();
    seedInflatedDays(store);

    installTx(store);
    const body = submissionBody(CORRECTED_SCAN);
    mockSubmit(body);
    const response = await post(body);
    const json = await response.json();

    expect(response.status).toBe(200);
    // Covered days adopt the snapshot outright, even though both decreased.
    expect(storedClientTokens(store, "2026-10-07", "devin-cli")).toBe(234_000_000);
    expect(storedClientTokens(store, "2026-10-08", "devin-cli")).toBe(455_000_000);
    // The day the snapshot does not report is kept at its stored value.
    expect(storedClientTokens(store, "2026-10-06", "devin-cli")).toBe(88_000_000);
    expect(storedTokens(store)).toBe(777_000_000);
    expect(json.metrics.totalTokens).toBe(777_000_000);
    // Model-level cells were rewritten too.
    const day8 = store.days.find((day) => day.date === "2026-10-08")!;
    expect(day8.sourceBreakdown["devin-cli"].models["swe-2-high"].tokens).toBe(
      455_000_000,
    );
    // The transition stamped the persisted generation.
    expect(store.device.parserVersions["devin-cli"]).toBe(2);
    expect(json.warnings?.some((warning: string) => warning.includes("Recounted"))).toBe(true);
  });

  it("freezes generation-1 submissions so the old parser cannot keep inflating", async () => {
    const store = newStore();
    seedInflatedDays(store);

    installTx(store);
    const body = submissionBody(
      [{ date: "2026-10-08", tokens: 2_000_000_000, cost: 2000 }],
      { version: 1 },
    );
    mockSubmit(body);
    const response = await post(body);
    const json = await response.json();

    expect(response.status).toBe(200);
    expect(storedTokens(store)).toBe(1_602_000_000);
    expect(storedClientTokens(store, "2026-10-08", "devin-cli")).toBe(1_000_000_000);
    expect(store.device.parserVersions["devin-cli"]).toBeUndefined();
    expect(json.warnings?.some((warning: string) => warning.includes("Ignored"))).toBe(true);
  });

  it("does not recount a second time once the transition state persists", async () => {
    const store = newStore();
    seedInflatedDays(store);

    installTx(store);
    const first = submissionBody(CORRECTED_SCAN);
    mockSubmit(first);
    expect((await post(first)).status).toBe(200);
    expect(storedTokens(store)).toBe(777_000_000);

    // A second generation-2 full snapshot at an even lower total enters the
    // incremental ledger: the credited high-water is the recounted baseline.
    installTx(store);
    const second = submissionBody([
      { date: "2026-10-08", tokens: 100_000_000, cost: 100 },
    ]);
    mockSubmit(second);
    const secondResponse = await post(second);
    const secondJson = await secondResponse.json();

    expect(secondResponse.status).toBe(200);
    expect(storedTokens(store)).toBe(777_000_000);
    expect(secondJson.warnings?.some((warning: string) =>
      warning.includes("fewer tokens than this device's credited baseline"),
    )).toBe(true);
  });

  it("suppresses new growth while the scan sits below the credited baseline, then credits the excess", async () => {
    const store = newStore();
    seedInflatedDays(store);

    installTx(store);
    const first = submissionBody(CORRECTED_SCAN);
    mockSubmit(first);
    expect((await post(first)).status).toBe(200);
    expect(storedTokens(store)).toBe(777_000_000);

    // The preserved 10-06 day (88M) stays in the credited ledger, but the
    // scanner no longer reports it, so the scan's lifetime (700M) is below
    // the credited baseline (777M): growth on a brand-new day is suppressed
    // until the scan exceeds it — the same monotonic rule as before.
    installTx(store);
    const second = submissionBody([
      ...CORRECTED_SCAN,
      { date: "2026-10-09", tokens: 11_000_000, cost: 11 },
    ]);
    mockSubmit(second);
    expect((await post(second)).status).toBe(200);
    expect(storedClientTokens(store, "2026-10-09", "devin-cli")).toBeUndefined();
    expect(storedTokens(store)).toBe(777_000_000);

    // Once the scan's lifetime exceeds the credited baseline, the excess is
    // credited newest-first; the 10-09 cell has the only positive capacity.
    installTx(store);
    const third = submissionBody([
      ...CORRECTED_SCAN,
      { date: "2026-10-09", tokens: 101_000_000, cost: 101 },
    ]);
    mockSubmit(third);
    expect((await post(third)).status).toBe(200);
    expect(storedClientTokens(store, "2026-10-09", "devin-cli")).toBe(13_000_000);
    expect(storedTokens(store)).toBe(790_000_000);
  });

  it("keeps the client's stored lifetime cost when the recounting snapshot is incomplete", async () => {
    const store = newStore();
    // Small numbers keep the floor math readable: stored lifetime $60, of
    // which $10 sits on the uncovered day and must not be re-added to the
    // covered cells the recount rewrites.
    seedDay(store, "2026-10-06", { "devin-cli": devinCell(1_000, 10) });
    seedDay(store, "2026-10-07", { "devin-cli": devinCell(2_000, 20) });
    seedDay(store, "2026-10-08", { "devin-cli": devinCell(3_000, 30) });

    installTx(store);
    const body = submissionBody(
      [
        { date: "2026-10-07", tokens: 900, cost: 5 },
        { date: "2026-10-08", tokens: 1200, cost: 8 },
      ],
      { costIsComplete: false },
    );
    mockSubmit(body);
    const response = await post(body);

    expect(response.status).toBe(200);
    // Tokens are recounted down; the incomplete snapshot may not lower the
    // credited lifetime cost, so the covered cells' $13 is topped up with the
    // $37 deficit to reach their $50 share of the floor, while the preserved
    // day's $10 is counted exactly once.
    expect(storedTokens(store)).toBe(1_000 + 900 + 1_200);
    expect(storedCost(store)).toBeCloseTo(60, 4);
    for (const date of ["2026-10-07", "2026-10-08"]) {
      const cell = store.days.find((day) => day.date === date)!
        .sourceBreakdown["devin-cli"];
      expect(cell.provenance?.costIsComplete).toBe(false);
    }
    // The incomplete floor lands on nested model costs, not just the client
    // aggregate, so model-level reads agree with the client total.
    const day8 = store.days.find((day) => day.date === "2026-10-08")!;
    const devin8 = day8.sourceBreakdown["devin-cli"];
    expect(devin8.models["swe-2-high"].cost).toBeCloseTo(devin8.cost, 4);
  });
});
