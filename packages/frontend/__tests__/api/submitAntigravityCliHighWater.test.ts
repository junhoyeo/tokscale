import { readFileSync } from "node:fs";
import { resolve } from "node:path";

import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

import { SUPPORTED_VERSIONED_PARSERS } from "../../src/lib/db/parserHighWater";
import type { ClientBreakdownData } from "../../src/lib/db/helpers";

// End-to-end regression for the Antigravity CLI re-attribution.
//
// The CLI used to date every turn of a conversation at the session start, and
// now dates each turn by the timestamp of the generation that produced it. A
// user who submitted under the old dating and rescans after upgrading sends
// the SAME lifetime usage spread across the days it actually ran.
//
// The per-day merge guard defends each stored (day, client) cell against a
// decrease, so on its own it pins the session-start day at its full total and
// inserts the later days on top: the stored lifetime total gains exactly what
// moved, permanently. Registering `antigravity-cli` in
// SUPPORTED_VERSIONED_PARSERS bounds each submission by the device/client
// lifetime high-water instead, so a pure re-attribution credits nothing.
//
// This is a route harness in the style of submitTimezoneResplitRepro: it runs
// the real merge/high-water code against a stateful transaction double that
// records the route's daily_breakdown writes, replays them into an in-memory
// table, and derives the later aggregate read from that table. The device row
// round-trips `parser_versions`/`parser_states` between the two POSTs, which
// is what lets the second submit see the first submit's high-water.
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
  const executedSqlArgs: unknown[] = [];
  const selectedColumns: Array<Record<string, unknown>> = [];

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
    select: vi.fn((columns: Record<string, unknown>) => {
      selectedColumns.push(columns);
      return makeAwaitableBuilder(selectResult(columns, store));
    }),
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
  return { executedSqlArgs, selectedColumns };
}

function submissionBody(
  client: string,
  days: Array<{ date: string; tokens: number; messages: number }>,
  retentionFloor?: string,
) {
  const dates = days.map((day) => day.date).sort();
  return {
    device: { id: "dev_1", name: "Device one" },
    meta: {
      generatedAt: "2026-08-10T00:00:00Z",
      version: "4.13.0",
      dateRange: { start: dates[0], end: dates[dates.length - 1] },
    },
    // The CLI declares generation 1 for every client but Copilot, so a real
    // Antigravity CLI submit carries exactly this.
    scanScope: {
      parserVersions: { [client]: 1 },
      fullHistory: true,
      ...(retentionFloor
        ? { retentionFloors: { [client]: retentionFloor } }
        : {}),
    },
    summary: { clients: [client] },
    years: [],
    contributions: days.map((day) => ({
      date: day.date,
      clients: [
        {
          client,
          modelId: "gemini-3-pro",
          tokens: {
            input: day.tokens,
            output: 0,
            cacheRead: 0,
            cacheWrite: 0,
            reasoning: 0,
          },
          cost: day.tokens / 1000,
          messages: day.messages,
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
  mockState.validateSubmission.mockReset();
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

// One 12-turn, 240,000-token conversation that ran from the evening of
// 2026-08-07 into 08-09. The old parser dated every turn at the session start;
// the new one dates each turn by its generation timestamp. Re-dating moves
// turns between days -- it does not create them -- so both layouts carry the
// same 240,000 tokens and the same 12 messages.
const SESSION_START_DATING = [
  { date: "2026-08-07", tokens: 240_000, messages: 12 },
];
const PER_GENERATION_DATING = [
  { date: "2026-08-07", tokens: 40_000, messages: 2 },
  { date: "2026-08-08", tokens: 120_000, messages: 6 },
  { date: "2026-08-09", tokens: 80_000, messages: 4 },
];

async function submitOldThenNew(client: string) {
  const store = newStore();

  installTx(store);
  const oldBody = submissionBody(client, SESSION_START_DATING);
  mockSubmit(oldBody);
  const first = await post(oldBody);
  expect(first.status).toBe(200);
  const firstJson = await first.json();

  installTx(store);
  const newBody = submissionBody(client, PER_GENERATION_DATING);
  mockSubmit(newBody);
  const second = await post(newBody);
  expect(second.status).toBe(200);
  const secondJson = await second.json();

  return { store, firstJson, secondJson };
}

describe("POST /api/submit antigravity-cli re-attribution high-water", () => {
  it("registers antigravity-cli at the generation the CLI declares", () => {
    // Both re-attributions move a token to a different day and never change
    // the lifetime total, so no installed generation has to be frozen out.
    // Registering anything above 1 would instead freeze every submit from
    // every shipped CLI until the two are bumped in lockstep.
    expect(SUPPORTED_VERSIONED_PARSERS["antigravity-cli"]).toBe(1);
  });

  it("does not raise the stored total when a session is re-dated per generation", async () => {
    const { store, firstJson, secondJson } = await submitOldThenNew(
      "antigravity-cli",
    );

    // The old-dating submit stores the whole session on its start day and
    // establishes the lifetime high-water at that total.
    expect(firstJson.metrics.totalTokens).toBe(240_000);

    // The re-dated rescan carries the SAME 240,000 tokens, so the high-water
    // credits nothing: the session-start row is preserved untouched and the
    // generation days add no rows at all.
    expect(secondJson.metrics.totalTokens).toBe(240_000);
    expect(storedTokens(store)).toBe(240_000);
    expect(store.days.map((day) => day.date)).toEqual(["2026-08-07"]);
    expect(store.days[0].sourceBreakdown["antigravity-cli"].tokens).toBe(
      240_000,
    );

    // The high-water state persisted on the device row is what the next
    // submit bounds against.
    expect(store.device.parserVersions["antigravity-cli"]).toBe(1);
    expect(
      (store.device.parserStates["antigravity-cli"] as {
        aggregate: { tokens: number };
      }).aggregate.tokens,
    ).toBe(240_000);
  });

  it("still credits genuinely new antigravity-cli usage after the re-dating", async () => {
    const { store } = await submitOldThenNew("antigravity-cli");

    installTx(store);
    const grown = submissionBody("antigravity-cli", [
      ...PER_GENERATION_DATING,
      { date: "2026-08-10", tokens: 55_000, messages: 3 },
    ]);
    mockSubmit(grown);
    const response = await post(grown);
    expect(response.status).toBe(200);
    const json = await response.json();

    // Only the real growth is credited, on the day it happened.
    expect(json.metrics.totalTokens).toBe(295_000);
    expect(
      store.days.find((day) => day.date === "2026-08-10")?.sourceBreakdown[
        "antigravity-cli"
      ].tokens,
    ).toBe(55_000);
  });

  it("credits later growth even when the original stored cell is still larger", async () => {
    const { store } = await submitOldThenNew("antigravity-cli");
    const grown = submissionBody("antigravity-cli", [
      { date: "2026-08-07", tokens: 60_000, messages: 3 },
      { date: "2026-08-08", tokens: 120_000, messages: 6 },
      { date: "2026-08-09", tokens: 80_000, messages: 4 },
    ]);

    installTx(store);
    mockSubmit(grown);
    const response = await post(grown);
    expect(response.status).toBe(200);
    const json = await response.json();

    expect(json.metrics.totalTokens).toBe(260_000);
    expect(storedTokens(store)).toBe(260_000);
    expect(
      store.days.find((day) => day.date === "2026-08-07")?.sourceBreakdown[
        "antigravity-cli"
      ].tokens,
    ).toBe(240_000);

    const state = store.device.parserStates["antigravity-cli"] as {
      aggregate: { tokens: number };
      days: Record<string, { tokens: number }>;
      observedDays: Record<string, { tokens: number }>;
    };
    expect(state.aggregate.tokens).toBe(260_000);
    expect(
      Object.values(state.days).reduce((sum, day) => sum + day.tokens, 0),
    ).toBe(260_000);
    expect(state.observedDays["2026-08-07"].tokens).toBe(60_000);

    // The credited ledger, not the parser's cellwise envelope, is the next
    // lifetime baseline. Replaying the same snapshot therefore adds nothing.
    installTx(store);
    mockSubmit(grown);
    const replay = await post(grown);
    expect(replay.status).toBe(200);
    expect((await replay.json()).metrics.totalTokens).toBe(260_000);
    expect(storedTokens(store)).toBe(260_000);
    expect(
      (store.device.parserStates["antigravity-cli"] as {
        aggregate: { tokens: number };
      }).aggregate.tokens,
    ).toBe(260_000);
  });

  it("reports the deficit when the store ages history out from under the high-water", async () => {
    const store = newStore();
    installTx(store);
    const first = submissionBody("antigravity-cli", SESSION_START_DATING);
    mockSubmit(first);
    expect((await post(first)).status).toBe(200);

    // The CLI keeps a rolling conversation store, so weeks later a full scan
    // no longer reaches the credited session at all: it reports 90,000
    // genuinely new tokens against a 240,000 credited lifetime.
    installTx(store);
    const aged = submissionBody("antigravity-cli", [
      { date: "2026-09-01", tokens: 90_000, messages: 5 },
    ]);
    mockSubmit(aged);
    const response = await post(aged);
    expect(response.status).toBe(200);
    const json = await response.json();

    // The bound holds, which is the point of the registry entry: a snapshot
    // below the credited lifetime is indistinguishable from a re-attribution.
    expect(json.metrics.totalTokens).toBe(240_000);
    expect(store.days.map((day) => day.date)).toEqual(["2026-08-07"]);

    // What must not happen silently. Without the warning this response is
    // identical to one that stored everything.
    expect(
      json.warnings.some(
        (warning: string) =>
          warning.includes("Added no Antigravity CLI tokens") &&
          warning.includes("150,000"),
      ),
    ).toBe(true);
  });

  it("credits the new window when the reported retention floor clears the stored days", async () => {
    const store = newStore();
    installTx(store);
    const first = submissionBody("antigravity-cli", SESSION_START_DATING);
    mockSubmit(first);
    expect((await post(first)).status).toBe(200);

    // Same aged-out scan as above, except the parser now reports how far back
    // its own store still reaches. The 2026-08-07 session is no longer on
    // disk, so it cannot be what these 90,000 tokens were moved from.
    installTx(store);
    const aged = submissionBody(
      "antigravity-cli",
      [{ date: "2026-09-01", tokens: 90_000, messages: 5 }],
      "2026-09-01",
    );
    mockSubmit(aged);
    const response = await post(aged);
    expect(response.status).toBe(200);
    const json = await response.json();

    expect(json.metrics.totalTokens).toBe(330_000);
    expect(store.days.map((day) => day.date)).toEqual([
      "2026-08-07",
      "2026-09-01",
    ]);
    expect(
      (json.warnings ?? []).some((warning: string) =>
        warning.includes("Added no Antigravity CLI tokens"),
      ),
    ).toBe(false);
  });

  it("warns about token deficit while retaining newly credited messages", async () => {
    const store = newStore();
    const first = submissionBody("antigravity-cli", SESSION_START_DATING);
    installTx(store);
    mockSubmit(first);
    expect((await post(first)).status).toBe(200);

    const smaller = submissionBody("antigravity-cli", [
      { date: "2026-09-01", tokens: 90_000, messages: 20 },
    ], "2026-08-07");
    installTx(store);
    mockSubmit(smaller);
    const response = await post(smaller);
    expect(response.status).toBe(200);
    const json = await response.json();
    const added = store.days.find((day) => day.date === "2026-09-01")!.sourceBreakdown["antigravity-cli"];
    expect(added.tokens).toBe(0);
    expect(added.messages).toBe(8);
    expect(json.metrics.totalTokens).toBe(240_000);
    expect(json.warnings).toEqual(expect.arrayContaining([
      expect.stringContaining("150,000"),
    ]));
    expect(json.warnings.some((warning: string) => warning.includes("Added no Antigravity CLI usage"))).toBe(false);
  });

  it("credits nothing when a re-attribution happens under a retention floor", async () => {
    const store = newStore();
    installTx(store);
    const first = submissionBody("antigravity-cli", SESSION_START_DATING);
    mockSubmit(first);
    expect((await post(first)).status).toBe(200);

    // The floor is derived from session start times still on disk, so a
    // re-dated session keeps its own start: the floor does not advance past
    // the credited day and the lifetime bound still sees all 240,000.
    installTx(store);
    const rescan = submissionBody(
      "antigravity-cli",
      PER_GENERATION_DATING,
      "2026-08-07",
    );
    mockSubmit(rescan);
    const response = await post(rescan);
    expect(response.status).toBe(200);
    const json = await response.json();

    expect(json.metrics.totalTokens).toBe(240_000);
  });

  it("ignores a retention floor contradicted by an older incoming day", async () => {
    const store = newStore();
    const first = submissionBody("antigravity-cli", [
      { date: "2026-08-07", tokens: 100_000, messages: 5 },
      { date: "2026-08-08", tokens: 200_000, messages: 5 },
    ]);
    installTx(store);
    mockSubmit(first);
    expect((await post(first)).status).toBe(200);

    const moved = submissionBody("antigravity-cli", [
      { date: "2026-08-07", tokens: 100_000, messages: 5 },
      { date: "2026-09-01", tokens: 200_000, messages: 5 },
    ], "2026-08-08");
    for (let replay = 0; replay < 2; replay++) {
      installTx(store);
      mockSubmit(moved);
      const response = await post(moved);
      expect(response.status).toBe(200);
      expect((await response.json()).metrics.totalTokens).toBe(300_000);
      expect(store.days.map(({ date }) => date)).toEqual(["2026-08-07", "2026-08-08"]);
    }
  });

  it("still inflates for a client that is legitimately not registered", async () => {
    // Claude's parser does not re-attribute submitted history, so it is not in
    // SUPPORTED_VERSIONED_PARSERS and takes the plain day-by-day merge path.
    // Feeding it the identical two payloads shows what that path does with a
    // re-attribution -- and therefore that the registry entry, not anything
    // else in the route, is what holds the antigravity-cli total flat.
    expect(SUPPORTED_VERSIONED_PARSERS.claude).toBeUndefined();

    const { store, firstJson, secondJson } = await submitOldThenNew("claude");

    expect(firstJson.metrics.totalTokens).toBe(240_000);

    // The merge guard refuses the decrease on the session-start day and the
    // two generation days are inserted on top: 240,000 of truth stored as
    // 440,000, with no correction path.
    expect(secondJson.metrics.totalTokens).toBe(440_000);
    expect(storedTokens(store)).toBe(440_000);
    expect(store.days.map((day) => day.date).sort()).toEqual([
      "2026-08-07",
      "2026-08-08",
      "2026-08-09",
    ]);
    expect(
      secondJson.warnings.some((warning: string) =>
        warning.includes("Preserved claude"),
      ),
    ).toBe(true);
  });
});

describe("POST /api/submit antigravity (IDE) re-attribution high-water", () => {
  // The IDE-backed client shares the sync that writes the artifacts, and #1151
  // stopped standalone usage rows falling back to the session-created date:
  // they are now correlated to trajectory-step timestamps. That is the same
  // re-attribution shape as the CLI client, on a different client id, so it
  // needs the same lifetime bound. The harness above is client-id agnostic,
  // so these reuse it directly.
  it("registers antigravity at the generation the CLI declares", () => {
    expect(SUPPORTED_VERSIONED_PARSERS.antigravity).toBe(1);
  });

  it("does not raise the stored total when standalone rows are re-dated", async () => {
    const { store, firstJson, secondJson } = await submitOldThenNew(
      "antigravity",
    );

    expect(firstJson.metrics.totalTokens).toBe(240_000);
    expect(secondJson.metrics.totalTokens).toBe(240_000);
    expect(storedTokens(store)).toBe(240_000);
    expect(store.days.map((day) => day.date)).toEqual(["2026-08-07"]);
    expect(store.device.parserVersions.antigravity).toBe(1);
  });

  it("still credits genuinely new antigravity usage after the re-dating", async () => {
    const { store } = await submitOldThenNew("antigravity");

    installTx(store);
    const grown = submissionBody("antigravity", [
      ...PER_GENERATION_DATING,
      { date: "2026-08-10", tokens: 55_000, messages: 3 },
    ]);
    mockSubmit(grown);
    const response = await post(grown);
    expect(response.status).toBe(200);
    const json = await response.json();

    expect(json.metrics.totalTokens).toBe(295_000);
    expect(
      store.days.find((day) => day.date === "2026-08-10")?.sourceBreakdown
        .antigravity.tokens,
    ).toBe(55_000);
  });
});

describe("POST /api/submit droid snapshot layout", () => {
  function snapshotBody(
    days: Array<{
      date: string;
      costIsComplete: boolean;
      models: Array<{ modelId: string; tokens: number; cost: number; messages: number }>;
    }>,
  ) {
    const body = submissionBody("droid", []);
    const dates = days.map(({ date }) => date).sort();
    return {
      ...body,
      meta: { ...body.meta, dateRange: { start: dates[0], end: dates[dates.length - 1] } },
      contributions: days.map((day) => ({
        date: day.date,
        totals: { costIsComplete: day.costIsComplete },
        clients: day.models.map((model) => ({
          client: "droid",
          modelId: model.modelId,
          tokens: { input: model.tokens, output: 0, cacheRead: 0, cacheWrite: 0, reasoning: 0 },
          cost: model.cost,
          messages: model.messages,
        })),
      })),
    };
  }

  async function submitSnapshot(store: Store, body: ReturnType<typeof snapshotBody>) {
    installTx(store);
    mockSubmit(body);
    const response = await post(body);
    expect(response.status).toBe(200);
    return response.json();
  }

  function expectSnapshotLayout(store: Store, body: ReturnType<typeof snapshotBody>) {
    for (const day of body.contributions) {
      const cell = store.days.find((stored) => stored.date === day.date)!.sourceBreakdown.droid;
      expect(Object.keys(cell.models).sort()).toEqual(day.clients.map(({ modelId }) => modelId).sort());
      for (const client of day.clients) {
        expect(cell.models[client.modelId]).toMatchObject({
          tokens: client.tokens.input,
          input: client.tokens.input,
          messages: client.messages,
        });
      }
      const models = Object.values(cell.models);
      expect(cell.tokens).toBe(models.reduce((sum, model) => sum + model.tokens, 0));
      expect(cell.messages).toBe(models.reduce((sum, model) => sum + model.messages, 0));
      expect(cell.cost).toBeCloseTo(models.reduce((sum, model) => sum + model.cost, 0), 8);
    }
  }

  const originalSnapshot = () => snapshotBody([{
    date: "2026-08-07",
    costIsComplete: false,
    models: [
      { modelId: "gemini-3-pro", tokens: 100_000, cost: 100, messages: 2 },
      { modelId: "unknown", tokens: 2, cost: 0, messages: 2 },
    ],
  }]);

  it.each([false, true])("reconciles overlapping partially priced days once per lifetime (legacy=%s)", async (legacy) => {
    const store = newStore();
    await submitSnapshot(store, originalSnapshot());
    if (legacy) {
      store.device.parserVersions = {};
      store.device.parserStates = {};
    }
    const rewritten = snapshotBody([
      { date: "2026-08-07", costIsComplete: false, models: [
        { modelId: "gemini-3-pro", tokens: 40_000, cost: 40, messages: 1 },
        { modelId: "unknown", tokens: 1, cost: 0, messages: 1 },
      ] },
      { date: "2026-08-08", costIsComplete: false, models: [
        { modelId: "gemini-3-pro", tokens: 60_000, cost: 60, messages: 1 },
        { modelId: "unknown", tokens: 1, cost: 0, messages: 1 },
      ] },
    ]);
    for (let replay = 0; replay < 3; replay++) {
      const json = await submitSnapshot(store, rewritten);
      expectSnapshotLayout(store, rewritten);
      expect(storedTokens(store)).toBe(100_002);
      expect(storedCost(store)).toBe(100);
      expect(json.metrics.totalCost).toBe(100);
      expect(store.days.every(({ sourceBreakdown }) => sourceBreakdown.droid.provenance?.costIsComplete === false)).toBe(true);
    }
  });

  it("removes moved models from an incomplete source day without duplicating their cost", async () => {
    const store = newStore();
    await submitSnapshot(store, originalSnapshot());
    const rewritten = snapshotBody([
      { date: "2026-08-07", costIsComplete: false, models: [
        { modelId: "unknown", tokens: 2, cost: 0, messages: 2 },
      ] },
      { date: "2026-08-08", costIsComplete: true, models: [
        { modelId: "gemini-3-pro", tokens: 100_000, cost: 100, messages: 2 },
      ] },
    ]);
    for (let replay = 0; replay < 2; replay++) {
      await submitSnapshot(store, rewritten);
      expectSnapshotLayout(store, rewritten);
      expect(storedCost(store)).toBe(100);
    }
  });

  it("keeps only the lifetime deficit across replays and clears it when pricing recovers", async () => {
    const store = newStore();
    await submitSnapshot(store, originalSnapshot());
    const rewritten = snapshotBody([
      { date: "2026-08-07", costIsComplete: false, models: [
        { modelId: "gemini-3-pro", tokens: 40_000, cost: 10, messages: 1 },
        { modelId: "unknown", tokens: 1, cost: 0, messages: 1 },
      ] },
      { date: "2026-08-08", costIsComplete: false, models: [
        { modelId: "gemini-3-pro", tokens: 60_000, cost: 20, messages: 1 },
        { modelId: "unknown", tokens: 1, cost: 0, messages: 1 },
      ] },
    ]);
    await submitSnapshot(store, rewritten);
    expect(storedCost(store)).toBeCloseTo(100, 8);
    expectSnapshotLayout(store, rewritten);
    const beforeReplay = structuredClone(store.days);
    await submitSnapshot(store, rewritten);
    expect(store.days).toEqual(beforeReplay);

    // A complete resubmit supplies the authoritative price and may correct
    // a prior floor downward, including cost inflated by the old merge.
    for (const day of rewritten.contributions) day.totals.costIsComplete = true;
    await submitSnapshot(store, rewritten);
    expect(storedCost(store)).toBe(30);
    expectSnapshotLayout(store, rewritten);
    expect(store.days.every(({ sourceBreakdown }) => sourceBreakdown.droid.provenance?.costIsComplete !== false)).toBe(true);
  });

  it("keeps rounding residuals on the same dates and models after new rows become updates", async () => {
    const store = newStore();
    const models = ["z-model", "a-model", "m-model", "b-model"].map((modelId) => ({
      modelId, tokens: 1, cost: 0, messages: 1,
    }));
    const original = snapshotBody([{
      date: "2026-08-07", costIsComplete: false,
      models: models.map((model, index) => ({ ...model, cost: index === 0 ? 0.0014 : 0 })),
    }]);
    await submitSnapshot(store, original);
    const rewritten = snapshotBody(["2026-08-07", "2026-08-08", "2026-08-09", "2026-08-10"].map((date) => ({
      date, costIsComplete: false, models,
    })));
    await submitSnapshot(store, rewritten);
    expect(storedCost(store)).toBeCloseTo(0.0014, 10);
    expectSnapshotLayout(store, rewritten);
    const beforeReplay = structuredClone(store.days);
    await submitSnapshot(store, rewritten);
    expect(store.days).toEqual(beforeReplay);
    rewritten.contributions.reverse();
    for (const day of rewritten.contributions) day.clients.reverse();
    await submitSnapshot(store, rewritten);
    expect(store.days).toEqual(beforeReplay);
    expectSnapshotLayout(store, rewritten);
  });

  it("allocates a lifetime deficit only to incomplete Droid days", async () => {
    const store = newStore();
    await submitSnapshot(store, originalSnapshot());
    const rewritten = snapshotBody([
      { date: "2026-08-07", costIsComplete: false, models: [
        { modelId: "unknown", tokens: 2, cost: 0, messages: 2 },
      ] },
      { date: "2026-08-08", costIsComplete: true, models: [
        { modelId: "gemini-3-pro", tokens: 100_000, cost: 60, messages: 2 },
      ] },
    ]);
    for (let replay = 0; replay < 3; replay++) {
      await submitSnapshot(store, rewritten);
      expectSnapshotLayout(store, rewritten);
      expect(storedCost(store)).toBe(100);
      const incomplete = store.days.find(({ date }) => date === "2026-08-07")!.sourceBreakdown.droid;
      const complete = store.days.find(({ date }) => date === "2026-08-08")!.sourceBreakdown.droid;
      expect(incomplete.cost).toBe(40);
      expect(incomplete.provenance?.costIsComplete).toBe(false);
      expect(complete.cost).toBe(60);
      expect(complete.models["gemini-3-pro"].cost).toBe(60);
      expect(complete.provenance?.costIsComplete).not.toBe(false);
    }
  });

  it("repairs previously retained model tokens and lets complete pricing correct inflated spend", async () => {
    const store = newStore();
    await submitSnapshot(store, originalSnapshot());
    // Model the already-persisted shape produced by the old day-floor merge.
    const oldCell = store.days[0].sourceBreakdown.droid;
    oldCell.models["moved-model"] = { ...oldCell.models["gemini-3-pro"], cost: 60 };
    oldCell.cost = 160;

    const incomplete = originalSnapshot();
    await submitSnapshot(store, incomplete);
    expectSnapshotLayout(store, incomplete);
    expect(storedCost(store)).toBe(160);
    const complete = originalSnapshot();
    complete.contributions[0].totals.costIsComplete = true;
    await submitSnapshot(store, complete);
    expectSnapshotLayout(store, complete);
    expect(storedCost(store)).toBe(100);
  });

  it("freezes an incomplete partial rescan after a replacement", async () => {
    const store = newStore();
    await submitSnapshot(store, originalSnapshot());
    await submitSnapshot(store, originalSnapshot());
    const beforePartial = structuredClone(store.days);
    const partial = snapshotBody([{
      date: "2026-08-09", costIsComplete: false,
      models: [{ modelId: "gemini-3-pro", tokens: 200_000, cost: 20, messages: 5 }],
    }]);
    partial.scanScope.fullHistory = false;
    await submitSnapshot(store, partial);
    expect(store.days).toEqual(beforePartial);
  });

  it("replaces stored Droid days when a full snapshot re-dates the same lifetime", async () => {
    const { store, firstJson, secondJson } = await submitOldThenNew("droid");

    expect(firstJson.metrics.totalTokens).toBe(240_000);
    expect(secondJson.metrics.totalTokens).toBe(240_000);
    expect(storedTokens(store)).toBe(240_000);
    expect(store.days.map((day) => day.date).sort()).toEqual([
      "2026-08-07",
      "2026-08-08",
      "2026-08-09",
    ]);
    expect(store.days.find((day) => day.date === "2026-08-07")?.sourceBreakdown.droid.tokens).toBe(
      40_000,
    );
    expect(store.days.find((day) => day.date === "2026-08-08")?.sourceBreakdown.droid.tokens).toBe(
      120_000,
    );
    expect(store.days.find((day) => day.date === "2026-08-09")?.sourceBreakdown.droid.tokens).toBe(
      80_000,
    );
    expect(
      secondJson.warnings.some((warning: string) =>
        warning.includes("Established the Droid parser generation"),
      ),
    ).toBe(false);
    expect(
      secondJson.warnings.some((warning: string) =>
        warning.includes("Rewrote Droid daily layout"),
      ),
    ).toBe(true);
  });

  it("keeps the stored Droid cost floor when a full unpriced snapshot replaces the layout", async () => {
    const store = newStore();

    installTx(store);
    const priced = submissionBody("droid", SESSION_START_DATING);
    mockSubmit(priced);
    expect((await post(priced)).status).toBe(200);

    installTx(store);
    const unpriced = submissionBody("droid", PER_GENERATION_DATING);
    for (const day of unpriced.contributions) {
      day.clients[0].cost = 0;
      (day as { totals?: { costIsComplete: boolean } }).totals = {
        costIsComplete: false,
      };
    }
    mockSubmit(unpriced);
    const response = await post(unpriced);
    expect(response.status).toBe(200);

    const startDay = store.days.find((day) => day.date === "2026-08-07");
    expect(startDay?.sourceBreakdown.droid.tokens).toBe(40_000);
    expect(Number(startDay?.sourceBreakdown.droid.cost)).toBe(40);
    expect(storedCost(store)).toBe(240);
    expect(startDay?.sourceBreakdown.droid.provenance?.costIsComplete).toBe(
      false,
    );
  });

  it("carries the stored Droid cost onto new days when an unpriced snapshot moves every token", async () => {
    const store = newStore();

    installTx(store);
    const priced = submissionBody("droid", SESSION_START_DATING);
    mockSubmit(priced);
    expect((await post(priced)).status).toBe(200);

    installTx(store);
    const moved = submissionBody("droid", [
      { date: "2026-08-08", tokens: 120_000, messages: 6 },
      { date: "2026-08-09", tokens: 120_000, messages: 6 },
    ]);
    for (const day of moved.contributions) {
      day.clients[0].cost = 0;
      (day as { totals?: { costIsComplete: boolean } }).totals = {
        costIsComplete: false,
      };
    }
    mockSubmit(moved);
    const response = await post(moved);
    expect(response.status).toBe(200);
    const json = await response.json();

    expect(store.days.map((day) => day.date).sort()).toEqual([
      "2026-08-08",
      "2026-08-09",
    ]);
    expect(storedCost(store)).toBe(240);
    expect(json.metrics.dateRange.start).toBe("2026-08-08");
    expect(json.metrics.dateRange.end).toBe("2026-08-09");
  });

  it("still credits genuinely new Droid usage after the layout rewrite", async () => {
    const { store } = await submitOldThenNew("droid");

    installTx(store);
    const grown = submissionBody("droid", [
      ...PER_GENERATION_DATING,
      { date: "2026-08-10", tokens: 55_000, messages: 3 },
    ]);
    mockSubmit(grown);
    const response = await post(grown);
    expect(response.status).toBe(200);
    const json = await response.json();

    expect(json.metrics.totalTokens).toBe(295_000);
    expect(
      store.days.find((day) => day.date === "2026-08-10")?.sourceBreakdown.droid
        .tokens,
    ).toBe(55_000);
  });
});

/**
 * A Droid day that also carries a client with no versioned parser. The
 * snapshot layout only ever moves the Droid cell, so the sibling's fate on a
 * day the snapshot no longer mentions is what the preserve warning describes.
 */
function droidAndClaudeBody(
  days: Array<{ date: string; droid?: number; claude?: number }>,
) {
  const dates = days.map((day) => day.date).sort();
  const entry = (client: string, tokens: number) => ({
    client,
    modelId: client === "droid" ? "gemini-3-pro" : "claude-sonnet-4",
    tokens: { input: tokens, output: 0, cacheRead: 0, cacheWrite: 0, reasoning: 0 },
    cost: tokens / 1000,
    messages: 2,
  });
  return {
    device: { id: "dev_1", name: "Device one" },
    meta: {
      generatedAt: "2026-08-10T00:00:00Z",
      version: "4.13.0",
      dateRange: { start: dates[0], end: dates[dates.length - 1] },
    },
    scanScope: { parserVersions: { droid: 1 }, fullHistory: true },
    summary: { clients: ["droid", "claude"] },
    years: [],
    contributions: days.map((day) => ({
      date: day.date,
      clients: [
        ...(day.droid ? [entry("droid", day.droid)] : []),
        ...(day.claude ? [entry("claude", day.claude)] : []),
      ],
    })),
  };
}

describe("POST /api/submit droid snapshot layout sibling clients", () => {
  it("does not floor complete Droid pricing because another client's day is incomplete", async () => {
    const store = newStore();
    const first = droidAndClaudeBody([
      { date: "2026-08-07", droid: 100_000 },
      { date: "2026-08-08", claude: 1_000 },
    ]);
    installTx(store);
    mockSubmit(first);
    expect((await post(first)).status).toBe(200);

    const corrected = droidAndClaudeBody([
      { date: "2026-08-07", droid: 100_000 },
      { date: "2026-08-08", claude: 1_000 },
    ]);
    corrected.contributions[0].clients[0].cost = 50;
    Object.assign(corrected.contributions[1], { totals: { costIsComplete: false } });
    installTx(store);
    mockSubmit(corrected);
    expect((await post(corrected)).status).toBe(200);
    const droid = store.days.find(({ date }) => date === "2026-08-07")!.sourceBreakdown.droid;
    expect(droid.cost).toBe(50);
    expect(droid.provenance?.costIsComplete).not.toBe(false);
    expect(store.days.find(({ date }) => date === "2026-08-08")!.sourceBreakdown.claude.cost).toBe(1);
  });

  it("does not report a sibling client as disappeared from a day the snapshot only emptied of Droid", async () => {
    const store = newStore();

    installTx(store);
    const first = droidAndClaudeBody([
      { date: "2026-08-07", droid: 240_000, claude: 10_000 },
    ]);
    mockSubmit(first);
    expect((await post(first)).status).toBe(200);

    installTx(store);
    const rewritten = droidAndClaudeBody([
      { date: "2026-08-08", droid: 120_000 },
      { date: "2026-08-09", droid: 120_000, claude: 10_000 },
    ]);
    mockSubmit(rewritten);
    const response = await post(rewritten);
    expect(response.status).toBe(200);
    const json = await response.json();

    // 08-07 is only in the write set because the Droid layout emptied it. The
    // day never disappeared for Claude -- the submission simply does not cover
    // it -- so the preserve warning must not fire.
    const startDay = store.days.find((day) => day.date === "2026-08-07");
    expect(startDay?.sourceBreakdown.claude?.tokens).toBe(10_000);
    expect(startDay?.sourceBreakdown.droid).toBeUndefined();
    expect(
      json.warnings.filter((warning: string) =>
        warning.includes("Preserved claude"),
      ),
    ).toEqual([]);
  });
});

describe("submission date range aggregate", () => {
  // This transaction double never enforces NOT NULL, so a green route test
  // proves nothing about a NULL date range. Pin the SQL expression instead:
  // `sql` is not mocked, so the fragment the route builds is a real drizzle
  // object and collectStrings reassembles it verbatim.
  function renderSql(fragment: unknown): string {
    const parts: string[] = [];
    collectStrings(fragment, parts);
    return parts.join("");
  }

  it("falls back to the unfiltered bounds, so a history with no token-bearing day is not NULL", async () => {
    const store = newStore();
    const { selectedColumns } = installTx(store);
    const body = submissionBody("droid", SESSION_START_DATING);
    mockSubmit(body);
    expect((await post(body)).status).toBe(200);

    const aggregate = selectedColumns.find(
      (columns) => "totalTokens" in columns && "dateStart" in columns,
    );
    expect(aggregate).toBeDefined();
    // MIN/MAX over the tokens filter alone is NULL for a user whose whole
    // stored history is legacy tokenless Cursor rows -- a shape validation
    // explicitly permits -- and both columns are NOT NULL.
    expect(renderSql(aggregate!.dateStart)).toBe(
      "COALESCE(MIN(CASE WHEN dailyBreakdown.tokens > 0 THEN dailyBreakdown.date END), MIN(dailyBreakdown.date))",
    );
    expect(renderSql(aggregate!.dateEnd)).toBe(
      "COALESCE(MAX(CASE WHEN dailyBreakdown.tokens > 0 THEN dailyBreakdown.date END), MAX(dailyBreakdown.date))",
    );
  });

  it("pins the NOT NULL columns the fallback exists for", () => {
    const migration = readFileSync(
      resolve(
        __dirname,
        "../../src/lib/db/migrations/0000_add_user_id_unique_constraint.sql",
      ),
      "utf8",
    );
    expect(migration).toContain('"date_start" date NOT NULL');
    expect(migration).toContain('"date_end" date NOT NULL');
  });
});
