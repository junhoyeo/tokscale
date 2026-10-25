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
// A lower snapshot alone is not evidence (a database that failed to open
// lowers it too), so the generation-2 CLI ships a per-day recount proof: what
// generation 1 counted from the same rows next to what generation 2 counts.
// The route subtracts only that proven over-count from each stored cell,
// never below the corrected count and never from usage the proof does not
// describe, on every full-history submit (`RECOUNT_PROOF_CLIENTS`).
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

type BodyDay = {
  date: string;
  tokens: number;
  cost?: number;
  /**
   * What parser generation 1 counted for this day from the same rows. When
   * present, the body carries a recount proof entry for the day whose
   * `corrected` side is exactly the submitted cell.
   */
  previous?: { tokens: number; cost?: number; messages?: number };
  /** Overrides the proof's corrected side, to model a proof built from other rows. */
  provenCorrected?: { tokens: number; cost?: number; messages?: number };
};

function proofCell(tokens: number, cost: number, messages: number) {
  return {
    tokens: { input: tokens, output: 0, cacheRead: 0, cacheWrite: 0, reasoning: 0 },
    cost,
    messages,
  };
}

function submissionBody(
  days: BodyDay[],
  options: {
    version?: number;
    fullHistory?: boolean;
    costIsComplete?: boolean;
    /**
     * Proof days the body does not date at all: generation 1 counted rows
     * there that generation 2 drops entirely, so the corrected side is empty.
     */
    allDuplicateDays?: Array<{ date: string; previous: { tokens: number; cost?: number; messages?: number } }>;
  } = {},
) {
  const version = options.version ?? SUPPORTED_VERSIONED_PARSERS["devin-cli"];
  const dates = days.map((day) => day.date).sort();
  const dayCost = (day: BodyDay) => day.cost ?? day.tokens / 1000;
  const proofDays = Object.fromEntries(
    [
      ...(options.allDuplicateDays ?? []).map((day) => ({
        date: day.date,
        tokens: 0,
        previous: day.previous,
        provenCorrected: { tokens: 0, cost: 0, messages: 0 },
      })),
      ...days,
    ]
      .filter((day) => day.previous)
      .map((day) => {
        const corrected = day.provenCorrected ?? { tokens: day.tokens, cost: dayCost(day), messages: 1 };
        return [
          day.date,
          {
            "swe-2-high": {
              previous: proofCell(
                day.previous!.tokens,
                day.previous!.cost ?? day.previous!.tokens / 1000,
                day.previous!.messages ?? 1,
              ),
              corrected: proofCell(
                corrected.tokens,
                corrected.cost ?? corrected.tokens / 1000,
                corrected.messages ?? 1,
              ),
            },
          },
        ];
      }),
  );
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
      ...(Object.keys(proofDays).length > 0
        ? { recountProofs: { "devin-cli": proofDays } }
        : {}),
    },
    summary: { clients: ["devin-cli"] },
    years: [],
    contributions: days.map((day) => ({
      date: day.date,
      totals: {
        tokens: day.tokens,
        cost: dayCost(day),
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
          cost: dayCost(day),
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

async function submit(store: Store, body: ReturnType<typeof submissionBody>) {
  installTx(store);
  mockSubmit(body);
  const response = await post(body);
  const json = await response.json();
  expect(response.status).toBe(200);
  return json as { warnings?: string[]; metrics: { totalTokens: number } };
}

// What the inflated generation-1 submissions left in the device: each day
// counts duplicate `message_nodes` rows, roughly 2.2x the real usage. A third
// stored day has no local data left, so no corrected snapshot reports it.
const INFLATED_DAYS = [
  { date: "2026-10-06", tokens: 88_000_000, cost: 88 },
  { date: "2026-10-07", tokens: 514_000_000, cost: 514 },
  { date: "2026-10-08", tokens: 1_000_000_000, cost: 1000 },
];
// The generation-2 scan of the same rows, with the proof of what generation 1
// counted from them.
const CORRECTED_SCAN: BodyDay[] = [
  { date: "2026-10-07", tokens: 234_000_000, cost: 234, previous: { tokens: 514_000_000, cost: 514 } },
  { date: "2026-10-08", tokens: 455_000_000, cost: 455, previous: { tokens: 1_000_000_000, cost: 1000 } },
];

function seedInflatedDays(store: Store) {
  for (const day of INFLATED_DAYS) {
    seedDay(store, day.date, { "devin-cli": devinCell(day.tokens, day.cost) });
  }
}

describe("POST /api/submit devin-cli recount proof", () => {
  it("registers devin-cli at the generation the deduplicating CLI declares", () => {
    // Generation 2 is the request_id recount. Registering 1 would accept
    // proofs from a generation that never computes them; registering above 2
    // freezes the only CLI that can heal the stored rows.
    expect(SUPPORTED_VERSIONED_PARSERS["devin-cli"]).toBe(2);
  });

  it("subtracts the proven over-count from covered stored cells and keeps uncovered days", async () => {
    const store = newStore();
    seedInflatedDays(store);

    const json = await submit(store, submissionBody(CORRECTED_SCAN));

    // Covered days drop by exactly what the proof shows generation 1 double
    // counted, which here lands them on the snapshot.
    expect(storedClientTokens(store, "2026-10-07", "devin-cli")).toBe(234_000_000);
    expect(storedClientTokens(store, "2026-10-08", "devin-cli")).toBe(455_000_000);
    // The day the snapshot does not report is kept at its stored value.
    expect(storedClientTokens(store, "2026-10-06", "devin-cli")).toBe(88_000_000);
    expect(storedTokens(store)).toBe(777_000_000);
    expect(storedCost(store)).toBeCloseTo(777, 4);
    expect(json.metrics.totalTokens).toBe(777_000_000);
    // Model-level cells were corrected too.
    const day8 = store.days.find((day) => day.date === "2026-10-08")!;
    expect(day8.sourceBreakdown["devin-cli"].models["swe-2-high"].tokens).toBe(455_000_000);
    expect(day8.sourceBreakdown["devin-cli"].provenance?.costIsComplete).toBeUndefined();
    // The transition stamped the persisted generation.
    expect(store.device.parserVersions["devin-cli"]).toBe(2);
    expect(json.warnings?.some((warning) => warning.includes("Corrected Devin CLI history on 2 day(s)"))).toBe(true);
  });

  it("does not lower a stored day on a lower snapshot without a proof", async () => {
    const store = newStore();
    seedInflatedDays(store);

    await submit(
      store,
      submissionBody(CORRECTED_SCAN.map(({ date, tokens, cost }) => ({ date, tokens, cost }))),
    );

    // A generation number is identity, not evidence: a lower total could as
    // well be a database that failed to open. The ordinary preserving
    // baseline applies.
    expect(storedTokens(store)).toBe(1_602_000_000);
    expect(store.device.parserVersions["devin-cli"]).toBe(2);
  });

  it("keeps stored usage from a source the scan could not read", async () => {
    // Two Devin databases fed the stored day: A = 400 tokens of which 50 are
    // duplicate rows, B = 600. Generation 2 reads A while B fails to open, so
    // the payload carries 350 for the date. Only A's proven 50 come off.
    const store = newStore();
    seedDay(store, "2026-10-08", { "devin-cli": devinCell(1_000, 1) });

    await submit(
      store,
      submissionBody([
        { date: "2026-10-08", tokens: 350, cost: 0.35, previous: { tokens: 400, cost: 0.4 } },
      ]),
    );
    expect(storedClientTokens(store, "2026-10-08", "devin-cli")).toBe(950);

    // When B is readable again (with 60 duplicates of its own) the proof
    // covers both databases and the day heals to the truth, without taking
    // A's duplicates off a second time.
    await submit(
      store,
      submissionBody([
        { date: "2026-10-08", tokens: 890, cost: 0.89, previous: { tokens: 1_000, cost: 1 } },
      ]),
    );
    expect(storedClientTokens(store, "2026-10-08", "devin-cli")).toBe(890);
    expect(storedCost(store)).toBeCloseTo(0.89, 4);
  });

  it("keeps generation-1 submissions compatible before the transition", async () => {
    const store = newStore();
    seedInflatedDays(store);

    const json = await submit(
      store,
      submissionBody([{ date: "2026-10-08", tokens: 2_000_000_000, cost: 2000 }], { version: 1 }),
    );

    // The receiver reaches production before a generation-2 CLI release. An
    // installed generation-1 CLI must retain the pre-deploy merge behavior
    // until this device starts its transition.
    expect(storedTokens(store)).toBe(2_602_000_000);
    expect(storedClientTokens(store, "2026-10-08", "devin-cli")).toBe(2_000_000_000);
    expect(store.device.parserVersions["devin-cli"]).toBeUndefined();
    expect((json.warnings ?? []).some((warning) => warning.includes("Ignored"))).toBe(false);
  });

  it("freezes generation 1 after the generation-2 transition", async () => {
    const store = newStore();
    seedInflatedDays(store);

    await submit(store, submissionBody(CORRECTED_SCAN));
    expect(storedTokens(store)).toBe(777_000_000);

    const json = await submit(
      store,
      submissionBody([{ date: "2026-10-08", tokens: 2_000_000_000, cost: 2000 }], { version: 1 }),
    );

    expect(storedTokens(store)).toBe(777_000_000);
    expect(storedClientTokens(store, "2026-10-08", "devin-cli")).toBe(455_000_000);
    expect(store.device.parserVersions["devin-cli"]).toBe(2);
    expect(json.warnings?.some((warning) => warning.includes("Ignored"))).toBe(true);
  });

  it("keeps a declared but empty Devin transition pending until history arrives", async () => {
    const store = newStore();
    seedInflatedDays(store);

    // Default submits declare every default parser. Model that real payload:
    // another client has data, while Devin is declared at generation 2 but
    // contributes no rows because its local database is absent or unreadable.
    const declaredOnly = submissionBody([{ date: "2026-10-08", tokens: 50, cost: 0.05 }]);
    declaredOnly.summary.clients = ["claude"];
    declaredOnly.contributions[0].clients[0].client = "claude";
    await submit(store, declaredOnly);

    expect(storedClientTokens(store, "2026-10-08", "devin-cli")).toBe(1_000_000_000);
    expect(store.device.parserVersions["devin-cli"]).toBe(2);
    expect(
      (store.device.parserStates["devin-cli"] as { baselineEstablished?: boolean })
        .baselineEstablished,
    ).toBe(false);

    // Once a generation-2 transition is pending, an old autosubmit binary may
    // not inflate the device while it waits for generation-2 data.
    await submit(
      store,
      submissionBody([{ date: "2026-10-08", tokens: 2_000_000_000, cost: 2000 }], { version: 1 }),
    );
    expect(storedClientTokens(store, "2026-10-08", "devin-cli")).toBe(1_000_000_000);

    // The first snapshot that actually carries Devin history applies its proof.
    await submit(store, submissionBody(CORRECTED_SCAN));
    expect(storedTokens(store)).toBe(777_000_050);
  });

  it("preserves stored Devin history across a partial generation-2 payload", async () => {
    const store = newStore();
    seedInflatedDays(store);

    await submit(
      store,
      submissionBody(
        [{ date: "2026-10-08", tokens: 1, cost: 0.001, previous: { tokens: 1_000_000_000 } }],
        { fullHistory: false },
      ),
    );
    expect(storedTokens(store)).toBe(1_602_000_000);
    expect(
      (store.device.parserStates["devin-cli"] as { baselineEstablished?: boolean })
        .baselineEstablished,
    ).toBe(false);

    await submit(store, submissionBody(CORRECTED_SCAN));
    expect(storedTokens(store)).toBe(777_000_000);
  });

  it("replays the same corrected snapshot idempotently", async () => {
    const store = newStore();
    seedInflatedDays(store);

    await submit(store, submissionBody(CORRECTED_SCAN));
    expect(storedTokens(store)).toBe(777_000_000);

    const json = await submit(store, submissionBody(CORRECTED_SCAN));
    expect(storedTokens(store)).toBe(777_000_000);
    expect(storedCost(store)).toBeCloseTo(777, 4);
    // Nothing left to remove, so nothing is reported as corrected.
    expect((json.warnings ?? []).some((warning) => warning.includes("Corrected"))).toBe(false);
  });

  it("suppresses new growth while the scan sits below the credited baseline, then credits the excess", async () => {
    const store = newStore();
    seedInflatedDays(store);

    await submit(store, submissionBody(CORRECTED_SCAN));
    expect(storedTokens(store)).toBe(777_000_000);

    // The preserved 10-06 day (88M) stays in the credited ledger, but the
    // scanner no longer reports it, so the scan's lifetime (700M) is below
    // the credited baseline (777M): growth on a brand-new day is suppressed
    // until the scan exceeds it — the same monotonic rule as before.
    await submit(
      store,
      submissionBody([...CORRECTED_SCAN, { date: "2026-10-09", tokens: 11_000_000, cost: 11 }]),
    );
    expect(storedClientTokens(store, "2026-10-09", "devin-cli")).toBeUndefined();
    expect(storedTokens(store)).toBe(777_000_000);

    // Once the scan's lifetime exceeds the credited baseline, the excess is
    // credited newest-first; the 10-09 cell has the only positive capacity.
    await submit(
      store,
      submissionBody([...CORRECTED_SCAN, { date: "2026-10-09", tokens: 101_000_000, cost: 101 }]),
    );
    expect(storedClientTokens(store, "2026-10-09", "devin-cli")).toBe(13_000_000);
    expect(storedTokens(store)).toBe(790_000_000);
  });

  it("ignores a proof that does not describe the submitted snapshot", async () => {
    const store = newStore();
    seedInflatedDays(store);

    const json = await submit(
      store,
      submissionBody([
        CORRECTED_SCAN[0],
        { ...CORRECTED_SCAN[1], provenCorrected: { tokens: 400_000_000 } },
      ]),
    );

    expect(storedClientTokens(store, "2026-10-07", "devin-cli")).toBe(234_000_000);
    expect(storedClientTokens(store, "2026-10-08", "devin-cli")).toBe(1_000_000_000);
    expect(json.warnings?.some((warning) => warning.includes("Ignored the Devin CLI recount proof for 1 day(s)"))).toBe(true);
  });

  it("floors cost on an incomplete recount, then corrects it exactly on the next complete snapshot", async () => {
    const store = newStore();
    // Stored lifetime $60: $10 on the uncovered day, $20 and $30 on the two
    // the corrected scan reports.
    seedDay(store, "2026-10-06", { "devin-cli": devinCell(1_000, 10) });
    seedDay(store, "2026-10-07", { "devin-cli": devinCell(2_000, 20) });
    seedDay(store, "2026-10-08", { "devin-cli": devinCell(3_000, 30) });
    const corrected: BodyDay[] = [
      { date: "2026-10-07", tokens: 900, cost: 5, previous: { tokens: 2_000, cost: 20 } },
      { date: "2026-10-08", tokens: 1_200, cost: 8, previous: { tokens: 3_000, cost: 30 } },
    ];

    const first = await submit(store, submissionBody(corrected, { costIsComplete: false }));

    // Tokens are corrected; the incomplete snapshot may not lower the stored
    // cost, so each covered cell keeps its stored cost as a floor and is
    // tagged, while the uncovered day is untouched.
    expect(storedTokens(store)).toBe(1_000 + 900 + 1_200);
    expect(storedCost(store)).toBeCloseTo(60, 4);
    // The response reports what was written, not what the proof would have
    // allowed: the floored cost was not removed.
    expect(
      first.warnings?.some((warning: string) =>
        warning.includes("removed 2,900 tokens and $0.0000"),
      ),
    ).toBe(true);
    for (const date of ["2026-10-07", "2026-10-08"]) {
      const cell = store.days.find((day) => day.date === date)!.sourceBreakdown["devin-cli"];
      expect(cell.provenance?.costIsComplete).toBe(false);
      expect(cell.models["swe-2-high"].cost).toBeCloseTo(cell.cost, 4);
    }

    // The same layout with complete pricing restates each covered cell's
    // cost: the proven over-count comes off and the tag clears, so the
    // duplicate-derived cost inflation does not become permanent.
    await submit(store, submissionBody(corrected, { costIsComplete: true }));

    expect(storedTokens(store)).toBe(1_000 + 900 + 1_200);
    expect(storedCost(store)).toBeCloseTo(10 + 5 + 8, 4);
    for (const date of ["2026-10-07", "2026-10-08"]) {
      const cell = store.days.find((day) => day.date === date)!.sourceBreakdown["devin-cli"];
      expect(cell.provenance?.costIsComplete).toBeUndefined();
      expect(cell.models["swe-2-high"].cost).toBeCloseTo(cell.cost, 4);
    }
    const day8 = store.days.find((day) => day.date === "2026-10-08")!;
    expect(day8.sourceBreakdown["devin-cli"].cost).toBeCloseTo(8, 4);
  });

  it("heals a stored day whose every row the corrected generation drops", async () => {
    const store = newStore();
    // Generation 1 dated 1,000 tokens on the 6th from rows that are all
    // request_id duplicates of the 7th (a response persisted across
    // midnight). Generation 2 counts nothing on the 6th, so the snapshot
    // never dates it; only the proof can say why the stored row is empty.
    seedDay(store, "2026-10-06", { "devin-cli": devinCell(1_000, 10) });
    seedDay(store, "2026-10-07", { "devin-cli": devinCell(2_000, 20) });

    const json = await submit(
      store,
      submissionBody(
        [{ date: "2026-10-07", tokens: 2_000, cost: 20, previous: { tokens: 2_000, cost: 20 } }],
        { allDuplicateDays: [{ date: "2026-10-06", previous: { tokens: 1_000, cost: 10 } }] },
      ),
    );

    expect(storedClientTokens(store, "2026-10-06", "devin-cli")).toBeUndefined();
    expect(storedClientTokens(store, "2026-10-07", "devin-cli")).toBe(2_000);
    expect(storedTokens(store)).toBe(2_000);
    expect(storedCost(store)).toBeCloseTo(20, 4);
    expect(
      json.warnings?.some((warning: string) =>
        warning.includes("Corrected Devin CLI history on 1 day(s)") &&
        warning.includes("removed 1,000 tokens and $10.0000"),
      ),
    ).toBe(true);
  });

  it("refuses a proof whose corrected cost sits below the submitted snapshot", async () => {
    const store = newStore();
    seedDay(store, "2026-10-07", { "devin-cli": devinCell(2_000, 20) });

    // The token side reproduces the snapshot exactly, but the proof claims
    // the corrected rows cost nothing while the snapshot prices them at $9.
    // Accepting it would floor the stored cost under what this scan reports.
    const json = await submit(
      store,
      submissionBody([
        {
          date: "2026-10-07",
          tokens: 900,
          cost: 9,
          previous: { tokens: 2_000, cost: 20 },
          provenCorrected: { tokens: 900, cost: 0 },
        },
      ]),
    );

    expect(storedClientTokens(store, "2026-10-07", "devin-cli")).toBe(2_000);
    expect(storedCost(store)).toBeCloseTo(20, 4);
    expect(
      json.warnings?.some((warning: string) =>
        warning.includes("Ignored the Devin CLI recount proof for 1 day(s)"),
      ),
    ).toBe(true);
  });
});
