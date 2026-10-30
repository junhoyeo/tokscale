import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

import {
  SUPPORTED_VERSIONED_PARSERS,
  type ParserClientHighWaterState,
} from "../../src/lib/db/parserHighWater";
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
  users: {
    id: "users.id",
    leaderboardHidden: "users.leaderboardHidden",
    updatedAt: "users.updatedAt",
  },
  moderationActions: {
    id: "moderationActions.id",
    targetUserId: "moderationActions.targetUserId",
    targetUsername: "moderationActions.targetUsername",
    actorUserId: "moderationActions.actorUserId",
    actorUsername: "moderationActions.actorUsername",
    action: "moderationActions.action",
    reason: "moderationActions.reason",
    createdAt: "moderationActions.createdAt",
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

function recountCell(
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
  if (keys.has("leaderboardHidden")) return [{ leaderboardHidden: false }];
  if (keys.has("action") && keys.has("actorUsername")) return [];
  throw new Error(`unexpected SELECT shape: ${[...keys].join(",")}`);
}

function makeAwaitableBuilder(result: unknown) {
  const builder = {
    from: vi.fn(() => builder),
    where: vi.fn(() => builder),
    for: vi.fn(() => builder),
    limit: vi.fn(() => builder),
    orderBy: vi.fn(() => builder),
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
        where: vi.fn(() => ({
          returning: vi.fn(() => Promise.resolve([])),
          then: (resolve: (value: unknown) => unknown) => Promise.resolve(resolve(undefined)),
        })),
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

type RecountClientCase = {
  client: string;
  modelId: string;
  label: string;
};

const DEVIN_RECOUNT_CASE: RecountClientCase = {
  client: "devin-cli",
  modelId: "swe-2-high",
  label: "Devin CLI",
};
const CURSOR_RECOUNT_CASE: RecountClientCase = {
  client: "cursor",
  modelId: "gemini-4-argon-medium",
  label: "Cursor",
};

type BodyDay = {
  date: string;
  tokens: number;
  cost?: number;
  messages?: number;
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
    recountClient?: RecountClientCase;
  } = {},
) {
  const recountClient = options.recountClient ?? DEVIN_RECOUNT_CASE;
  const version =
    options.version ?? SUPPORTED_VERSIONED_PARSERS[recountClient.client];
  const dates = days.map((day) => day.date).sort();
  const dayCost = (day: BodyDay) => day.cost ?? day.tokens / 1000;
  const proofDays = Object.fromEntries(
    [
      ...(options.allDuplicateDays ?? []).map((day) => ({
        date: day.date,
        tokens: 0,
        messages: 0,
        previous: day.previous,
        provenCorrected: { tokens: 0, cost: 0, messages: 0 },
      })),
      ...days,
    ]
      .filter((day) => day.previous)
      .map((day) => {
        const corrected = day.provenCorrected ?? {
          tokens: day.tokens,
          cost: dayCost(day),
          messages: day.messages ?? 1,
        };
        return [
          day.date,
          {
            [recountClient.modelId]: {
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
      parserVersions: { [recountClient.client]: version },
      fullHistory: options.fullHistory ?? true,
      ...(Object.keys(proofDays).length > 0
        ? { recountProofs: { [recountClient.client]: proofDays } }
        : {}),
    },
    summary: { clients: [recountClient.client] },
    years: [],
    contributions: days.map((day) => ({
      date: day.date,
      totals: {
        tokens: day.tokens,
        cost: dayCost(day),
        messages: day.messages ?? 1,
        costIsComplete: options.costIsComplete ?? true,
      },
      clients: [
        {
          client: recountClient.client,
          modelId: recountClient.modelId,
          tokens: {
            input: day.tokens,
            output: 0,
            cacheRead: 0,
            cacheWrite: 0,
            reasoning: 0,
          },
          cost: dayCost(day),
          messages: day.messages ?? 1,
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
    seedDay(store, day.date, { "devin-cli": recountCell(day.tokens, day.cost) });
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
    // duplicate rows, B = 600. Generation 2 reads A while B is gone, so the
    // payload carries 350 for the date and the proof explains only 400 of the
    // stored 1,000. The receiver keeps no record of past corrections, so it
    // must not take A's 50 off here: every later submit with this same proof
    // would take them off again and wear B's 600 down to 350.
    const store = newStore();
    seedDay(store, "2026-10-08", { "devin-cli": recountCell(1_000, 1) });

    const partial = () =>
      submissionBody([
        { date: "2026-10-08", tokens: 350, cost: 0.35, previous: { tokens: 400, cost: 0.4 } },
      ]);
    for (let replay = 0; replay < 3; replay += 1) {
      await submit(store, partial());
      expect(storedClientTokens(store, "2026-10-08", "devin-cli")).toBe(1_000);
    }

    // B is readable again (with 60 duplicates of its own), so the proof now
    // covers both databases. The device is past its transition, and a later
    // proof never lowers counts: the day stays as stored.
    const bothDatabases = () =>
      submissionBody([
        { date: "2026-10-08", tokens: 890, cost: 0.89, previous: { tokens: 1_000, cost: 1 } },
      ]);
    await submit(store, bothDatabases());
    expect(storedClientTokens(store, "2026-10-08", "devin-cli")).toBe(1_000);
    expect(storedCost(store)).toBeCloseTo(1, 4);

    // A device whose transition reads both databases heals the day then.
    const healed = newStore();
    seedDay(healed, "2026-10-08", { "devin-cli": recountCell(1_000, 1) });
    await submit(healed, bothDatabases());
    expect(storedClientTokens(healed, "2026-10-08", "devin-cli")).toBe(890);
    expect(storedCost(healed)).toBeCloseTo(0.89, 4);
  });

  it("keeps a corrected day when one of its databases is later deleted", async () => {
    // Databases A and B fed the stored day: 200 tokens under generation 1
    // from each, 100 under generation 2 from each. The first submit reads both.
    const store = newStore();
    seedDay(store, "2026-10-08", { "devin-cli": recountCell(400, 0.4) });

    await submit(
      store,
      submissionBody([
        { date: "2026-10-08", tokens: 200, cost: 0.2, previous: { tokens: 400, cost: 0.4 } },
      ]),
    );
    expect(storedClientTokens(store, "2026-10-08", "devin-cli")).toBe(200);

    // B is deleted. A's proof explains only A, yet its previous side (200
    // tokens, 1 message) equals the corrected day exactly; B's 100 must stay.
    for (let replay = 0; replay < 2; replay += 1) {
      await submit(
        store,
        submissionBody([
          { date: "2026-10-08", tokens: 100, cost: 0.1, previous: { tokens: 200, cost: 0.2 } },
        ]),
      );
      expect(storedClientTokens(store, "2026-10-08", "devin-cli")).toBe(200);
      expect(storedCost(store)).toBeCloseTo(0.2, 4);
    }
  });

  it("never lowers a complete cost whose counts only equal the corrected ones", async () => {
    // The stored day holds database B's 100 tokens at $0.15. Database A's
    // proof corrects 200 tokens to 100 at $0.10: the same counts, but nothing
    // in it explains B's cost.
    const store = newStore();
    seedDay(store, "2026-10-08", { "devin-cli": recountCell(100, 0.15) });
    for (let submitIndex = 0; submitIndex < 2; submitIndex += 1) {
      await submit(
        store,
        submissionBody([
          { date: "2026-10-08", tokens: 100, cost: 0.1, previous: { tokens: 200, cost: 0.2, messages: 2 } },
        ]),
      );
      expect(storedClientTokens(store, "2026-10-08", "devin-cli")).toBe(100);
      expect(storedCost(store)).toBeCloseTo(0.15, 4);
    }
  });

  it("keeps the floor tag on a stored day the proof does not restate", async () => {
    // The stored day's $1.00 is a floor, and the proof explains only one of
    // the databases behind it, so the day is kept and its cost is still only
    // a lower bound after a complete snapshot.
    const store = newStore();
    const floored = recountCell(1_000, 1);
    floored.provenance = { schemaVersion: 1, messageCount: 1, modelCount: 1, costIsComplete: false };
    seedDay(store, "2026-10-08", { "devin-cli": floored });

    await submit(
      store,
      submissionBody([
        { date: "2026-10-08", tokens: 350, cost: 0.35, previous: { tokens: 400, cost: 0.4 } },
      ]),
    );

    const cell = store.days.find((day) => day.date === "2026-10-08")!.sourceBreakdown["devin-cli"];
    expect(cell.tokens).toBe(1_000);
    expect(cell.cost).toBeCloseTo(1, 4);
    expect(cell.provenance?.costIsComplete).toBe(false);
  });

  it("keeps the floor tag on a cell the transition corrects only in part", async () => {
    // swe-2-high holds generation-1 counts the proof explains; swe-1-5 is
    // not in the proof, and the cell's cost is a floor.
    const store = newStore();
    seedDay(store, "2026-10-08", {
      "devin-cli": {
        tokens: 250,
        cost: 0.5,
        input: 250,
        output: 0,
        cacheRead: 0,
        cacheWrite: 0,
        reasoning: 0,
        messages: 2,
        models: {
          "swe-2-high": recountCell(200, 0.3).models["swe-2-high"],
          "swe-1-5": recountCell(50, 0.2, 1, "swe-1-5").models["swe-1-5"],
        },
        provenance: { schemaVersion: 1, messageCount: 2, modelCount: 2, costIsComplete: false },
      },
    });

    await submit(
      store,
      submissionBody([
        { date: "2026-10-08", tokens: 100, cost: 0.1, previous: { tokens: 200, cost: 0.3 } },
      ]),
    );

    // swe-2-high is corrected; the proof says nothing about swe-1-5's cost,
    // so the cell keeps its floor tag even though this snapshot is complete.
    const cell = store.days.find((day) => day.date === "2026-10-08")!.sourceBreakdown["devin-cli"];
    expect(cell.models["swe-2-high"].tokens).toBe(100);
    expect(cell.models["swe-2-high"].cost).toBeCloseTo(0.1, 4);
    expect(cell.models["swe-1-5"].cost).toBeCloseTo(0.2, 4);
    expect(cell.tokens).toBe(150);
    expect(cell.provenance?.costIsComplete).toBe(false);
  });

  it("keeps the floor tag when the transition corrects counts but not an unexplained cost", async () => {
    // The stored $0.50 is a floor above what generation 1 priced these rows
    // at ($0.20), so the proof does not account for it and it is kept.
    const store = newStore();
    const floored = recountCell(200, 0.5);
    floored.provenance = { schemaVersion: 1, messageCount: 1, modelCount: 1, costIsComplete: false };
    seedDay(store, "2026-10-08", { "devin-cli": floored });

    await submit(
      store,
      submissionBody([
        { date: "2026-10-08", tokens: 100, cost: 0.1, previous: { tokens: 200, cost: 0.2 } },
      ]),
    );

    const cell = store.days.find((day) => day.date === "2026-10-08")!.sourceBreakdown["devin-cli"];
    expect(cell.tokens).toBe(100);
    expect(cell.cost).toBeCloseTo(0.5, 4);
    expect(cell.provenance?.costIsComplete).toBe(false);
  });

  it("clears the floor tag of a cell whose cost a complete proof already accounts for", async () => {
    // The floored cell already holds the corrected counts and cost, so the
    // proof changes no amount; it only shows the cost to be complete.
    const store = newStore();
    const floored = recountCell(100, 0.1);
    floored.provenance = { schemaVersion: 1, messageCount: 1, modelCount: 1, costIsComplete: false };
    seedDay(store, "2026-10-08", { "devin-cli": floored });

    await submit(
      store,
      submissionBody([
        { date: "2026-10-08", tokens: 100, cost: 0.1, previous: { tokens: 200, cost: 0.2, messages: 2 } },
      ]),
    );

    const cell = store.days.find((day) => day.date === "2026-10-08")!.sourceBreakdown["devin-cli"];
    expect(cell.tokens).toBe(100);
    expect(cell.cost).toBeCloseTo(0.1, 4);
    expect(cell.provenance?.costIsComplete).toBeUndefined();
  });

  it("never restates a floored cost whose counts are not the corrected ones", async () => {
    // Database B's floored $0.15 for 120 tokens; database A's proof corrects
    // 200 tokens to 100 at $0.10. Nothing in it describes B's cell.
    const store = newStore();
    const floored = recountCell(120, 0.15);
    floored.provenance = { schemaVersion: 1, messageCount: 1, modelCount: 1, costIsComplete: false };
    seedDay(store, "2026-10-08", { "devin-cli": floored });
    for (let submitIndex = 0; submitIndex < 2; submitIndex += 1) {
      await submit(
        store,
        submissionBody([
          { date: "2026-10-08", tokens: 100, cost: 0.1, previous: { tokens: 200, cost: 0.2, messages: 2 } },
        ]),
      );
      const cell = store.days.find((day) => day.date === "2026-10-08")!.sourceBreakdown["devin-cli"];
      expect(cell.tokens).toBe(120);
      expect(cell.cost).toBeCloseTo(0.15, 4);
      expect(cell.provenance?.costIsComplete).toBe(false);
    }
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

  it("floors cost on an incomplete recount, then corrects it on the next complete snapshot", async () => {
    const store = newStore();
    // Stored lifetime $60: $10 on the uncovered day, $20 and $30 on the two
    // the corrected scan reports.
    seedDay(store, "2026-10-06", { "devin-cli": recountCell(1_000, 10) });
    seedDay(store, "2026-10-07", { "devin-cli": recountCell(2_000, 20) });
    seedDay(store, "2026-10-08", { "devin-cli": recountCell(3_000, 30) });
    const corrected: BodyDay[] = [
      { date: "2026-10-07", tokens: 900, cost: 5, previous: { tokens: 2_000, cost: 20 } },
      { date: "2026-10-08", tokens: 1_200, cost: 8, previous: { tokens: 3_000, cost: 30 } },
    ];

    const first = await submit(store, submissionBody(corrected, { costIsComplete: false }));

    // Tokens are corrected; the incomplete snapshot may not lower the stored
    // cost, so each covered cell keeps its stored cost as a floor and is
    // tagged, while the uncovered day is untouched. The planner's ledger
    // keeps the same floors.
    expect(storedTokens(store)).toBe(1_000 + 900 + 1_200);
    expect(storedCost(store)).toBeCloseTo(60, 4);
    expect(
      first.warnings?.some((warning: string) =>
        warning.includes("removed 2,900 tokens and $0.0000"),
      ),
    ).toBe(true);
    const ledger = store.device.parserStates["devin-cli"] as ParserClientHighWaterState;
    for (const date of ["2026-10-07", "2026-10-08"]) {
      const cell = store.days.find((day) => day.date === date)!.sourceBreakdown["devin-cli"];
      expect(cell.provenance?.costIsComplete).toBe(false);
      expect(cell.models["swe-2-high"].cost).toBeCloseTo(cell.cost, 4);
      expect(ledger.days[date]?.cost).toBeCloseTo(cell.cost, 4);
    }

    // The same layout with complete pricing restates each covered cell's
    // cost: the proven over-count comes off and the tag clears, so the
    // duplicate-derived cost inflation does not become permanent.
    await submit(store, submissionBody(corrected, { costIsComplete: true }));

    expect(storedTokens(store)).toBe(1_000 + 900 + 1_200);
    expect(storedCost(store)).toBeCloseTo(10 + 5 + 8, 4);
    const healedLedger = store.device.parserStates["devin-cli"] as ParserClientHighWaterState;
    for (const date of ["2026-10-07", "2026-10-08"]) {
      const cell = store.days.find((day) => day.date === date)!.sourceBreakdown["devin-cli"];
      expect(cell.provenance?.costIsComplete).toBeUndefined();
      expect(cell.models["swe-2-high"].cost).toBeCloseTo(cell.cost, 4);
      expect(healedLedger.days[date]?.cost).toBeCloseTo(cell.cost, 4);
    }
    const day8 = store.days.find((day) => day.date === "2026-10-08")!;
    expect(day8.sourceBreakdown["devin-cli"].cost).toBeCloseTo(8, 4);
  });

  it("keeps a stored cost when the proof's cost rests on an estimate the submission zeroed", async () => {
    // An older CLI priced the stored rows at $0.15. This CLI can only
    // estimate them: the proof carries the estimate ($0.20 previous, $0.10
    // corrected) while the submission zeroed it and marked the day incomplete.
    const store = newStore();
    seedDay(store, "2026-10-08", { "devin-cli": recountCell(200, 0.15) });

    await submit(
      store,
      submissionBody(
        [
          {
            date: "2026-10-08",
            tokens: 100,
            cost: 0,
            previous: { tokens: 200, cost: 0.2 },
            provenCorrected: { tokens: 100, cost: 0.1 },
          },
        ],
        { costIsComplete: false },
      ),
    );

    const cell = store.days.find((day) => day.date === "2026-10-08")!.sourceBreakdown["devin-cli"];
    expect(cell.tokens).toBe(100);
    expect(cell.cost).toBeCloseTo(0.15, 4);
    expect(cell.provenance?.costIsComplete).toBe(false);
    const ledger = store.device.parserStates["devin-cli"] as ParserClientHighWaterState;
    expect(ledger.days["2026-10-08"]?.tokens).toBe(100);
    expect(ledger.days["2026-10-08"]?.cost).toBeCloseTo(0.15, 4);
  });

  it("keeps the planner ledger and the stored cost in step after an incomplete transition", async () => {
    const store = newStore();
    seedDay(store, "2026-10-08", { "devin-cli": recountCell(2_000, 20) });
    const ledgerCost = () =>
      (store.device.parserStates["devin-cli"] as ParserClientHighWaterState).days["2026-10-08"]?.cost;

    await submit(
      store,
      submissionBody(
        [{ date: "2026-10-08", tokens: 900, cost: 5, previous: { tokens: 2_000, cost: 20 } }],
        { costIsComplete: false },
      ),
    );
    expect(storedCost(store)).toBeCloseTo(20, 4);
    expect(ledgerCost()).toBeCloseTo(20, 4);

    // The day grows by 100 tokens costing $1. The increment's cost is the
    // snapshot's cost above the ledger's, so the ledger must hold the stored
    // floor: both stay at $20 while the day's counts are not the corrected
    // ones the floor was kept for.
    const grown = () =>
      submissionBody([
        { date: "2026-10-08", tokens: 1_000, cost: 6, previous: { tokens: 2_100, cost: 21 } },
      ]);
    await submit(store, grown());
    expect(storedClientTokens(store, "2026-10-08", "devin-cli")).toBe(1_000);
    expect(storedCost(store)).toBeCloseTo(20, 4);
    expect(ledgerCost()).toBeCloseTo(20, 4);

    // The next complete snapshot of the same layout restates the floor in
    // both.
    await submit(store, grown());
    expect(storedCost(store)).toBeCloseTo(6, 4);
    expect(ledgerCost()).toBeCloseTo(6, 4);
    const cell = store.days.find((day) => day.date === "2026-10-08")!.sourceBreakdown["devin-cli"];
    expect(cell.provenance?.costIsComplete).toBeUndefined();
  });

  it("corrects a device's only stored day when every row on it is a duplicate", async () => {
    // The device stored only the 8th. Its rows are all duplicates of a
    // response the corrected generation dates on the 9th, which the device
    // never stored. With the 8th emptied no stored usage is left, but the
    // stored row still has to go, or the 9th would count it a second time.
    const store = newStore();
    seedDay(store, "2026-10-08", { "devin-cli": recountCell(100, 0.1) });
    const body = (tokens: number) =>
      submissionBody(
        [{ date: "2026-10-09", tokens, cost: tokens / 1000, previous: { tokens, cost: tokens / 1000 } }],
        { allDuplicateDays: [{ date: "2026-10-08", previous: { tokens: 100, cost: 0.1 } }] },
      );

    await submit(store, body(50));
    expect(storedClientTokens(store, "2026-10-08", "devin-cli")).toBeUndefined();
    expect(storedClientTokens(store, "2026-10-09", "devin-cli")).toBe(50);
    const ledger = store.device.parserStates["devin-cli"] as ParserClientHighWaterState;
    expect(ledger.aggregate.tokens).toBe(50);

    // Later growth on the 9th is credited once.
    await submit(store, body(150));
    expect(storedTokens(store)).toBe(150);
    expect(storedCost(store)).toBeCloseTo(0.15, 4);
  });

  it("keeps a duplicate day's cost as a floor until its survivor's day is priced", async () => {
    // As above, but the 9th could not be priced, and the older CLI priced the
    // 8th's rows at $0.15 (the proof's own estimate says $0.20). Their
    // survivor's price is unknown, so the emptied 8th keeps its cost as a
    // floor.
    const store = newStore();
    seedDay(store, "2026-10-08", { "devin-cli": recountCell(100, 0.15) });
    const body = (costIsComplete: boolean) =>
      submissionBody(
        [{ date: "2026-10-09", tokens: 50, cost: 0.05, previous: { tokens: 50, cost: 0.05 } }],
        {
          costIsComplete,
          allDuplicateDays: [{ date: "2026-10-08", previous: { tokens: 100, cost: 0.2 } }],
        },
      );
    const ledger = () => store.device.parserStates["devin-cli"] as ParserClientHighWaterState;

    await submit(store, body(false));
    const floored = store.days.find((day) => day.date === "2026-10-08")?.sourceBreakdown["devin-cli"];
    expect(floored?.tokens).toBe(0);
    expect(floored?.messages).toBe(0);
    expect(floored?.cost).toBeCloseTo(0.15, 4);
    expect(floored?.provenance?.costIsComplete).toBe(false);
    expect(ledger().aggregate.tokens).toBe(50);
    expect(ledger().days["2026-10-08"]?.cost ?? 0).toBeCloseTo(0.15, 4);

    // Once the survivor's day is priced, the survivor carries the cost and
    // the floor goes with the emptied day.
    await submit(store, body(true));
    expect(storedClientTokens(store, "2026-10-08", "devin-cli")).toBeUndefined();
    expect(storedTokens(store)).toBe(50);
    expect(storedCost(store)).toBeCloseTo(0.05, 4);
    expect(ledger().days["2026-10-08"]?.cost ?? 0).toBeCloseTo(0, 4);
  });

  it("tags a kept duplicate cost as a floor when its survivor's day is not priced", async () => {
    // On the 8th, swe-2-high has survivors there; every swe-1-5 row is a
    // duplicate of a survivor dated the 9th, which this submission could not
    // price. The 8th itself is priced completely.
    const store = newStore();
    seedDay(store, "2026-10-08", {
      "devin-cli": {
        tokens: 300,
        cost: 0.35,
        input: 300,
        output: 0,
        cacheRead: 0,
        cacheWrite: 0,
        reasoning: 0,
        messages: 2,
        models: {
          "swe-2-high": recountCell(200, 0.2).models["swe-2-high"],
          "swe-1-5": recountCell(100, 0.15, 1, "swe-1-5").models["swe-1-5"],
        },
      },
    });
    const body = (ninthIsComplete: boolean) => {
      const submission = submissionBody([
        { date: "2026-10-08", tokens: 100, cost: 0.1, previous: { tokens: 200, cost: 0.2 } },
        { date: "2026-10-09", tokens: 50, cost: 0.05, previous: { tokens: 50, cost: 0.05 } },
      ]);
      // The 9th also carries swe-1-5's survivor, whose estimate an unpriced
      // 9th zeroes.
      submission.contributions[1].totals.costIsComplete = ninthIsComplete;
      submission.contributions[1].clients.push({
        client: "devin-cli",
        modelId: "swe-1-5",
        tokens: { input: 100, output: 0, cacheRead: 0, cacheWrite: 0, reasoning: 0 },
        cost: ninthIsComplete ? 0.15 : 0,
        messages: 1,
      });
      type ProofDay = Record<
        string,
        { previous: ReturnType<typeof proofCell>; corrected: ReturnType<typeof proofCell> }
      >;
      const proofs = submission.scanScope.recountProofs!["devin-cli"] as Record<string, ProofDay>;
      proofs["2026-10-08"]["swe-1-5"] = { previous: proofCell(100, 0.15, 1), corrected: proofCell(0, 0, 0) };
      proofs["2026-10-09"]["swe-1-5"] = { previous: proofCell(100, 0.15, 1), corrected: proofCell(100, 0.15, 1) };
      return submission;
    };
    const day8 = () => store.days.find((day) => day.date === "2026-10-08")!.sourceBreakdown["devin-cli"];

    await submit(store, body(false));
    // The counts are corrected; with a day of the submission unpriced, every
    // cost stays, tagged, until the survivors' price is known.
    expect(day8().tokens).toBe(100);
    expect(day8().models["swe-1-5"].tokens).toBe(0);
    expect(day8().models["swe-1-5"].cost).toBeCloseTo(0.15, 4);
    expect(day8().models["swe-2-high"].cost).toBeCloseTo(0.2, 4);
    expect(day8().cost).toBeCloseTo(0.35, 4);
    expect(day8().provenance?.costIsComplete).toBe(false);
    const ledger = store.device.parserStates["devin-cli"] as ParserClientHighWaterState;
    expect(ledger.days["2026-10-08"]?.cost).toBeCloseTo(0.35, 4);

    // A submission that prices every day restates the floors and clears them:
    // swe-1-5's cost moves to the 9th, where its survivor is.
    await submit(store, body(true));
    expect(day8().tokens).toBe(100);
    expect(day8().cost).toBeCloseTo(0.1, 4);
    expect(day8().provenance?.costIsComplete).toBeUndefined();
    const day9 = store.days.find((day) => day.date === "2026-10-09")!.sourceBreakdown["devin-cli"];
    expect(day9.models["swe-1-5"].cost).toBeCloseTo(0.15, 4);
    expect(storedCost(store)).toBeCloseTo(0.1 + 0.05 + 0.15, 4);
  });

  it("keeps a day's cost while a duplicate's survivor on another day is unpriced", async () => {
    // The 8th's 200 stored tokens are request A (100, $0.05) and a duplicate
    // of request B (100, $0.10), a response persisted across midnight whose
    // survivor is dated the 9th. The 8th is priced; the 9th is not, so the
    // submission zeroed its estimate. Lowering the 8th to $0.05 now would
    // lose B's $0.10 while the 9th holds $0.
    const store = newStore();
    seedDay(store, "2026-10-08", { "devin-cli": recountCell(200, 0.15) });
    const body = (ninthIsComplete: boolean) => {
      const submission = submissionBody([
        { date: "2026-10-08", tokens: 100, cost: 0.05, previous: { tokens: 200, cost: 0.15 } },
        {
          date: "2026-10-09",
          tokens: 100,
          cost: ninthIsComplete ? 0.1 : 0,
          previous: { tokens: 100, cost: 0.1 },
          provenCorrected: { tokens: 100, cost: 0.1 },
        },
      ]);
      submission.contributions[1].totals.costIsComplete = ninthIsComplete;
      return submission;
    };
    const cellOn = (date: string) => store.days.find((day) => day.date === date)!.sourceBreakdown["devin-cli"];

    await submit(store, body(false));
    expect(cellOn("2026-10-08").tokens).toBe(100);
    expect(cellOn("2026-10-08").cost).toBeCloseTo(0.15, 4);
    expect(cellOn("2026-10-08").provenance?.costIsComplete).toBe(false);
    expect(storedCost(store)).toBeCloseTo(0.15, 4);
    const ledger = store.device.parserStates["devin-cli"] as ParserClientHighWaterState;
    expect(ledger.days["2026-10-08"]?.cost).toBeCloseTo(0.15, 4);

    // Once every day is priced, B's cost moves to the 9th.
    await submit(store, body(true));
    expect(cellOn("2026-10-08").cost).toBeCloseTo(0.05, 4);
    expect(cellOn("2026-10-09").cost).toBeCloseTo(0.1, 4);
    expect(storedCost(store)).toBeCloseTo(0.15, 4);
    expect(cellOn("2026-10-08").provenance?.costIsComplete).toBeUndefined();
    expect(cellOn("2026-10-09").provenance?.costIsComplete).toBeUndefined();
  });

  it("keeps a duplicate-only model's cost while a survivor under another model is unpriced", async () => {
    // Every swe-1-5 row on the 8th is a copy of a response whose survivor the
    // corrected generation keeps on the 9th under swe-2-high. The 9th is
    // unpriced, so the 8th keeps its $0.15 until it is.
    const store = newStore();
    seedDay(store, "2026-10-08", { "devin-cli": recountCell(100, 0.15, 1, "swe-1-5") });
    const body = (ninthIsComplete: boolean) => {
      const submission = submissionBody([
        {
          date: "2026-10-09",
          tokens: 100,
          cost: ninthIsComplete ? 0.15 : 0,
          previous: { tokens: 100, cost: 0.15 },
          provenCorrected: { tokens: 100, cost: 0.15 },
        },
      ]);
      submission.contributions[0].totals.costIsComplete = ninthIsComplete;
      const proofs = submission.scanScope.recountProofs!["devin-cli"] as Record<
        string,
        Record<string, { previous: ReturnType<typeof proofCell>; corrected: ReturnType<typeof proofCell> }>
      >;
      proofs["2026-10-08"] = {
        "swe-1-5": { previous: proofCell(100, 0.15, 1), corrected: proofCell(0, 0, 0) },
      };
      return submission;
    };

    await submit(store, body(false));
    const floored = store.days.find((day) => day.date === "2026-10-08")?.sourceBreakdown["devin-cli"];
    expect(floored?.tokens).toBe(0);
    expect(floored?.cost).toBeCloseTo(0.15, 4);
    expect(floored?.provenance?.costIsComplete).toBe(false);

    await submit(store, body(true));
    expect(storedClientTokens(store, "2026-10-08", "devin-cli")).toBeUndefined();
    expect(storedCost(store)).toBeCloseTo(0.15, 4);
  });

  it("heals a stored day whose every row the corrected generation drops", async () => {
    const store = newStore();
    // Generation 1 dated 1,000 tokens on the 6th from rows that are all
    // request_id duplicates of the 7th (a response persisted across
    // midnight). Generation 2 counts nothing on the 6th, so the snapshot
    // never dates it; only the proof can say why the stored row is empty.
    seedDay(store, "2026-10-06", { "devin-cli": recountCell(1_000, 10) });
    seedDay(store, "2026-10-07", { "devin-cli": recountCell(2_000, 20) });

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
    seedDay(store, "2026-10-07", { "devin-cli": recountCell(2_000, 20) });

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

describe("POST /api/submit cursor recount proof", () => {
  it("corrects stored Cursor snapshots through the generic route protocol", async () => {
    const store = newStore();
    seedDay(store, "2026-10-08", {
      [CURSOR_RECOUNT_CASE.client]: recountCell(
        1_000,
        10,
        4,
        CURSOR_RECOUNT_CASE.modelId,
      ),
    });

    const json = await submit(
      store,
      submissionBody(
        [
          {
            date: "2026-10-08",
            tokens: 400,
            cost: 4,
            messages: 1,
            previous: { tokens: 1_000, cost: 10, messages: 4 },
          },
        ],
        { recountClient: CURSOR_RECOUNT_CASE },
      ),
    );

    const cell = store.days.find((day) => day.date === "2026-10-08")!
      .sourceBreakdown[CURSOR_RECOUNT_CASE.client];
    expect(cell.tokens).toBe(400);
    expect(cell.cost).toBeCloseTo(4, 4);
    expect(cell.messages).toBe(1);
    expect(cell.models[CURSOR_RECOUNT_CASE.modelId].tokens).toBe(400);
    expect(cell.models[CURSOR_RECOUNT_CASE.modelId].messages).toBe(1);
    expect(store.device.parserVersions[CURSOR_RECOUNT_CASE.client]).toBe(2);
    expect(
      json.warnings?.some((warning: string) =>
        warning.includes(
          `Corrected ${CURSOR_RECOUNT_CASE.label} history on 1 day(s)`,
        ),
      ),
    ).toBe(true);
  });

  it("keeps Cursor usage the proof cannot explain across replays", async () => {
    // The cache no longer holds every snapshot this device once submitted:
    // the proof's rows explain 500 of the stored 1,000 tokens. Taking the
    // proven 400 off on every submit would wear the day down to 100.
    const store = newStore();
    seedDay(store, "2026-10-08", {
      [CURSOR_RECOUNT_CASE.client]: recountCell(
        1_000,
        10,
        4,
        CURSOR_RECOUNT_CASE.modelId,
      ),
    });
    const partial = () =>
      submissionBody(
        [
          {
            date: "2026-10-08",
            tokens: 100,
            cost: 1,
            messages: 1,
            previous: { tokens: 500, cost: 5, messages: 2 },
          },
        ],
        { recountClient: CURSOR_RECOUNT_CASE },
      );

    for (let replay = 0; replay < 3; replay += 1) {
      await submit(store, partial());
      const cell = store.days.find((day) => day.date === "2026-10-08")!
        .sourceBreakdown[CURSOR_RECOUNT_CASE.client];
      expect(cell.tokens).toBe(1_000);
      expect(cell.cost).toBeCloseTo(10, 4);
      expect(cell.messages).toBe(4);
    }
    expect(store.device.parserVersions[CURSOR_RECOUNT_CASE.client]).toBe(2);
  });

  it("keeps a Cursor day's cost while a moved snapshot's survivor is unpriced", async () => {
    // Real Cursor snapshots keep their event's day (#1544). If one did not,
    // every stored row on the 8th could be an earlier snapshot of an event
    // whose survivor is dated the 9th, which this submission could not price.
    // The proof shows the model without a survivor on the 8th, so its cost
    // waits for the 9th to be priced.
    const store = newStore();
    seedDay(store, "2026-10-08", {
      [CURSOR_RECOUNT_CASE.client]: recountCell(100, 0.15, 1, CURSOR_RECOUNT_CASE.modelId),
    });
    const options = (costIsComplete: boolean) => ({
      recountClient: CURSOR_RECOUNT_CASE,
      costIsComplete,
      allDuplicateDays: [{ date: "2026-10-08", previous: { tokens: 100, cost: 0.2, messages: 1 } }],
    });
    const ninth = (cost: number) => [
      {
        date: "2026-10-09",
        tokens: 100,
        cost,
        messages: 1,
        previous: { tokens: 100, cost: 0.15, messages: 1 },
        provenCorrected: { tokens: 100, cost: 0.15, messages: 1 },
      },
    ];

    await submit(store, submissionBody(ninth(0), options(false)));
    const floored = store.days.find((day) => day.date === "2026-10-08")?.sourceBreakdown[CURSOR_RECOUNT_CASE.client];
    expect(floored?.tokens).toBe(0);
    expect(floored?.cost).toBeCloseTo(0.15, 4);
    expect(floored?.provenance?.costIsComplete).toBe(false);

    await submit(store, submissionBody(ninth(0.15), options(true)));
    expect(storedClientTokens(store, "2026-10-08", CURSOR_RECOUNT_CASE.client)).toBeUndefined();
    expect(storedCost(store)).toBeCloseTo(0.15, 4);
  });

  it("keeps a corrected Cursor day when one of its accounts is logged out", async () => {
    // Accounts A and B both used Cursor on 2026-10-08: 200 tokens each under
    // generation 1, 100 each once snapshots are collapsed. The first
    // generation-2 submit reads both caches and corrects the day to 200.
    const store = newStore();
    seedDay(store, "2026-10-08", {
      [CURSOR_RECOUNT_CASE.client]: recountCell(400, 4, 4, CURSOR_RECOUNT_CASE.modelId),
    });
    const options = { recountClient: CURSOR_RECOUNT_CASE };
    await submit(
      store,
      submissionBody(
        [{ date: "2026-10-08", tokens: 200, cost: 2, messages: 2, previous: { tokens: 400, cost: 4, messages: 4 } }],
        options,
      ),
    );
    expect(storedClientTokens(store, "2026-10-08", CURSOR_RECOUNT_CASE.client)).toBe(200);

    // `tokscale cursor logout` archives B's cache. A's proof still counts A's
    // duplicates under generation 1, and its previous side (200 tokens, 2
    // messages) equals the corrected day exactly; B's 100 must stay.
    for (let replay = 0; replay < 2; replay += 1) {
      await submit(
        store,
        submissionBody(
          [{ date: "2026-10-08", tokens: 100, cost: 1, messages: 1, previous: { tokens: 200, cost: 2, messages: 2 } }],
          options,
        ),
      );
      expect(storedClientTokens(store, "2026-10-08", CURSOR_RECOUNT_CASE.client)).toBe(200);
      expect(storedCost(store)).toBeCloseTo(2, 4);
    }
  });

  it("keeps crediting new Cursor usage after an account's cache is archived", async () => {
    // 2026-10-01 came from an account the user logged out of: its cache is
    // archived and no longer scanned. 2026-10-08 is the account still in use.
    const store = newStore();
    seedDay(store, "2026-10-01", {
      [CURSOR_RECOUNT_CASE.client]: recountCell(5_000, 50, 10, CURSOR_RECOUNT_CASE.modelId),
    });
    seedDay(store, "2026-10-08", {
      [CURSOR_RECOUNT_CASE.client]: recountCell(1_000, 10, 4, CURSOR_RECOUNT_CASE.modelId),
    });
    const options = { recountClient: CURSOR_RECOUNT_CASE };
    const inUse = { date: "2026-10-08", tokens: 400, cost: 4, messages: 1, previous: { tokens: 1_000, cost: 10, messages: 4 } };

    await submit(store, submissionBody([inUse], options));
    expect(storedClientTokens(store, "2026-10-08", CURSOR_RECOUNT_CASE.client)).toBe(400);
    expect(storedClientTokens(store, "2026-10-01", CURSOR_RECOUNT_CASE.client)).toBe(5_000);

    const json = await submit(
      store,
      submissionBody([inUse, { date: "2026-10-09", tokens: 300, cost: 3, messages: 1 }], options),
    );
    expect(storedClientTokens(store, "2026-10-09", CURSOR_RECOUNT_CASE.client)).toBe(300);
    expect(storedClientTokens(store, "2026-10-08", CURSOR_RECOUNT_CASE.client)).toBe(400);
    expect(storedClientTokens(store, "2026-10-01", CURSOR_RECOUNT_CASE.client)).toBe(5_000);
    expect(
      (json.warnings ?? []).some((warning: string) => warning.includes("Added no Cursor tokens")),
    ).toBe(false);
  });
});

describe("POST /api/submit legacy Cursor tokenless cost", () => {
  it("credits a newly discovered legacy day once after the parser baseline", async () => {
    const store = newStore();
    const options = {
      recountClient: { ...CURSOR_RECOUNT_CASE, modelId: "premium-tool-call" },
    };
    const original = { date: "2025-04-14", tokens: 0, cost: 0.02, messages: 1 };
    const discovered = { date: "2025-04-15", tokens: 0, cost: 0.02, messages: 1 };
    await submit(store, submissionBody([original], options));
    expect(storedCost(store)).toBeCloseTo(0.02, 4);
    const rescan = [original, discovered].map((day) => ({
      ...day,
      previous: { tokens: day.tokens, cost: day.cost, messages: day.messages },
    }));
    for (let replay = 0; replay < 3; replay++) {
      await submit(store, submissionBody(rescan, options));
      const stored = store.days.find((day) => day.date === discovered.date)!
        .sourceBreakdown.cursor;
      const ledger = store.device.parserStates.cursor as ParserClientHighWaterState;
      expect(stored.messages).toBe(1);
      expect(stored.cost).toBeCloseTo(0.02, 4);
      expect(ledger.days[discovered.date]?.cost).toBeCloseTo(stored.cost, 4);
      expect(storedCost(store)).toBeCloseTo(0.04, 4);
    }
  });

  it.each([
    { modelId: "premium-tool-call", beforeTokens: 0, afterTokens: 0 },
    { modelId: "gemini-4-argon-medium", beforeTokens: 100, afterTokens: 200 },
  ])(
    "credits new $modelId usage without repricing or replaying its cost",
    async ({ modelId, beforeTokens, afterTokens }) => {
      const store = newStore();
      const options = { recountClient: { ...CURSOR_RECOUNT_CASE, modelId } };
      const date = "2025-04-15";
      await submit(
        store,
        submissionBody(
          [{ date, tokens: beforeTokens, cost: 0.02, messages: 1 }],
          options,
        ),
      );
      const grown = { date, tokens: afterTokens, cost: 0.05, messages: 2 };
      const scan = {
        ...grown,
        previous: {
          tokens: grown.tokens,
          cost: grown.cost,
          messages: grown.messages,
        },
      };
      for (let replay = 0; replay < 3; replay++) {
        await submit(store, submissionBody([scan], options));
        const stored = store.days.find((day) => day.date === date)!
          .sourceBreakdown.cursor;
        const ledger = store.device.parserStates.cursor as ParserClientHighWaterState;
        expect(stored.tokens).toBe(afterTokens);
        expect(stored.messages).toBe(2);
        expect(stored.cost).toBeCloseTo(0.05, 4);
        expect(ledger.days[date]?.cost).toBeCloseTo(stored.cost, 4);
      }
      // A new price without a new billed invocation/token is not new spend.
      await submit(
        store,
        submissionBody(
          [{ ...scan, cost: 0.09, previous: { ...scan.previous, cost: 0.09 } }],
          options,
        ),
      );
      expect(storedCost(store)).toBeCloseTo(0.05, 4);
    },
  );
});
