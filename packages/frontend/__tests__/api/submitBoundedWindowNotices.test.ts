import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

import type { ClientBreakdownData } from "../../src/lib/db/helpers";

// Route harness for the bounded-submit preserve notices (#1513 follow-up).
//
// A date-bounded submit ships its `scanScope.fullHistoryClients` unfiltered,
// so the payload dates days every other scanned client was scoped out of. On
// such a day the merge used to emit one "Preserved X because it disappeared"
// notice per scanned bounded-growth client, plus a "Preserved ... active
// time" notice, even though nothing about stored data changed — a weekly
// submit against a long exempt-client history printed hundreds of lines of
// noise. These tests pin the suppression to exactly that day class and prove
// the stored bytes are what the unsuppressed path writes, for bounded and
// unbounded submits alike.
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

function cell(tokens: number, cost: number, messages = 1, modelId = "m1"): ClientBreakdownData {
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

function seedDay(
  store: Store,
  date: string,
  breakdown: StoredBreakdown,
  activeTimeMs: number | null = null,
) {
  store.days.push({
    id: `stored-${date}`,
    date,
    timestampMs: null,
    activeTimeMs,
    sourceBreakdown: breakdown,
  });
}

function storedDay(store: Store, date: string): PersistedDay | undefined {
  return store.days.find((day) => day.date === date);
}

function aggregatesRow(store: Store) {
  const totalTokens = store.days.reduce(
    (total, day) =>
      total +
      Object.values(day.sourceBreakdown).reduce((sum, c) => sum + c.tokens, 0),
    0,
  );
  const dates = store.days
    .filter((day) => Object.values(day.sourceBreakdown).some((c) => c.tokens > 0))
    .map((day) => day.date)
    .sort();
  return {
    totalTokens,
    totalCost: store.days
      .reduce(
        (total, day) =>
          total +
          Object.values(day.sourceBreakdown).reduce((sum, c) => sum + (c.cost ?? 0), 0),
        0,
      )
      .toFixed(4),
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
    then: (resolve: (value: unknown) => unknown) => Promise.resolve(resolve(result)),
  };
  return builder;
}

function isDailyBreakdownInsert(text: string): boolean {
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
    const update = text.includes("UPDATE daily_breakdown AS d SET");
    if (!insert && !update) return;

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
            store.device.parserVersions = payload.parserVersions as Record<string, number>;
            store.device.parserStates = payload.parserStates as Record<string, unknown>;
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
    transaction: vi.fn(async (callback: (sp: typeof tx) => Promise<unknown>) => callback(tx)),
  };
  mockState.db.transaction.mockImplementation(
    async (callback: (transaction: typeof tx) => Promise<unknown>) => callback(tx),
  );
}

type BodyDay = {
  date: string;
  activeTimeMs?: number;
  clients: Array<{ client: string; tokens: number; cost?: number }>;
};

function submissionBody(
  days: BodyDay[],
  options: {
    clients: string[];
    parserVersions?: Record<string, number>;
    fullHistory: boolean;
    fullHistoryClients?: string[];
  },
) {
  const dates = days.map((day) => day.date).sort();
  const dayCost = (c: { tokens: number; cost?: number }) => c.cost ?? c.tokens / 1000;
  return {
    device: { id: "dev_1", name: "Device one" },
    meta: {
      generatedAt: "2026-02-15T00:00:00Z",
      version: "4.20.0",
      dateRange: { start: dates[0], end: dates[dates.length - 1] },
    },
    scanScope: {
      parserVersions: options.parserVersions ?? {},
      fullHistory: options.fullHistory,
      ...(options.fullHistoryClients ? { fullHistoryClients: options.fullHistoryClients } : {}),
    },
    summary: { clients: options.clients },
    years: [],
    contributions: days.map((day) => ({
      date: day.date,
      ...(day.activeTimeMs != null ? { activeTimeMs: day.activeTimeMs } : {}),
      totals: {
        tokens: day.clients.reduce((sum, c) => sum + c.tokens, 0),
        cost: day.clients.reduce((sum, c) => sum + dayCost(c), 0),
        messages: day.clients.length,
      },
      clients: day.clients.map((c) => ({
        client: c.client,
        modelId: "m1",
        tokens: { input: c.tokens, output: 0, cacheRead: 0, cacheWrite: 0, reasoning: 0 },
        cost: dayCost(c),
        messages: 1,
      })),
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

async function submit(store: Store, body: ReturnType<typeof submissionBody>) {
  installTx(store);
  mockSubmit(body);
  const response = await POST(
    new Request("http://localhost:3000/api/submit", {
      method: "POST",
      headers: {
        Authorization: "Bearer tt_valid",
        "Content-Type": "application/json",
      },
      body: JSON.stringify(body),
    }),
  );
  const json = (await response.json()) as { warnings?: string[] };
  expect(response.status).toBe(200);
  return json;
}

const disappeared = (warnings: string[] | undefined, client: string) =>
  (warnings ?? []).filter((w) => w.includes("disappeared") && w.includes(client));
const activeTimeNotices = (warnings: string[] | undefined) =>
  (warnings ?? []).filter((w) => w.includes("active time"));

describe("POST /api/submit bounded-window preserve notices", () => {
  it("suppresses per-day preserve notices on days covered only by declared clients, without changing stored bytes", async () => {
    const store = newStore();
    // An exempt-covered day outside the requested window: stored codex and
    // cursor cells plus day active time, which the bounded payload cannot
    // restate because both clients were scoped to the window.
    const codexCell = cell(500, 0.5);
    const cursorCell = cell(300, 0.3);
    seedDay(
      store,
      "2025-12-01",
      { droid: cell(1_000, 1), codex: codexCell, cursor: cursorCell },
      3_600_000,
    );

    const json = await submit(
      store,
      submissionBody(
        [
          // droid ships complete (declared), so this day is dated by the
          // payload with droid alone and a lower (partial) active time.
          { date: "2025-12-01", activeTimeMs: 600_000, clients: [{ client: "droid", tokens: 1_200 }] },
          // The requested window itself.
          {
            date: "2026-02-10",
            clients: [
              { client: "droid", tokens: 50 },
              { client: "codex", tokens: 200 },
              { client: "cursor", tokens: 100 },
            ],
          },
        ],
        {
          clients: ["droid", "codex", "cursor"],
          parserVersions: { droid: 1, codex: 1, cursor: 1 },
          fullHistory: false,
          fullHistoryClients: ["droid"],
        },
      ),
    );

    // The out-of-window day used to warn once per scanned bounded-growth
    // client plus once for active time; all of it is noise there.
    expect(disappeared(json.warnings, "codex")).toEqual([]);
    expect(disappeared(json.warnings, "cursor")).toEqual([]);
    expect(activeTimeNotices(json.warnings)).toEqual([]);

    // Stored data behaviour is unchanged: the preserve path still ran, so the
    // cells carry the same values (plus the derived provenance the
    // unsuppressed path has always added).
    const day = storedDay(store, "2025-12-01")!;
    expect(day.sourceBreakdown.codex.tokens).toBe(codexCell.tokens);
    expect(day.sourceBreakdown.codex.cost).toBe(codexCell.cost);
    expect(day.sourceBreakdown.codex.models).toEqual(codexCell.models);
    expect(day.sourceBreakdown.cursor.tokens).toBe(cursorCell.tokens);
    expect(day.sourceBreakdown.cursor.models).toEqual(cursorCell.models);
    // The in-window day was written normally.
    const inWindow = storedDay(store, "2026-02-10")!;
    expect(inWindow.sourceBreakdown.codex.tokens).toBe(200);
    expect(inWindow.sourceBreakdown.cursor.tokens).toBe(100);
  });

  it("keeps the notices on in-window days that carry bounded-growth clients", async () => {
    const store = newStore();
    seedDay(store, "2026-02-11", { codex: cell(300, 0.3), cursor: cell(700, 0.7) }, 3_600_000);

    const json = await submit(
      store,
      submissionBody(
        [
          // codex (not declared complete) is present, so this day is inside
          // the window and cursor's absence is the real disappeared signal.
          { date: "2026-02-11", activeTimeMs: 500_000, clients: [{ client: "codex", tokens: 350 }] },
        ],
        {
          clients: ["droid", "codex", "cursor"],
          parserVersions: { droid: 1, codex: 1, cursor: 1 },
          fullHistory: false,
          fullHistoryClients: ["droid"],
        },
      ),
    );

    expect(disappeared(json.warnings, "cursor")).toHaveLength(1);
    expect(activeTimeNotices(json.warnings)).toHaveLength(1);
    const day = storedDay(store, "2026-02-11")!;
    expect(day.sourceBreakdown.cursor.tokens).toBe(700);
    expect(day.sourceBreakdown.codex.tokens).toBe(350);
  });

  it("leaves bounded submits without a per-client declaration (older CLIs) unchanged", async () => {
    const store = newStore();
    seedDay(store, "2025-12-02", { droid: cell(1_000, 1), codex: cell(500, 0.5) }, 3_600_000);

    const json = await submit(
      store,
      submissionBody(
        [{ date: "2025-12-02", activeTimeMs: 600_000, clients: [{ client: "droid", tokens: 1_200 }] }],
        {
          clients: ["droid", "codex"],
          parserVersions: { droid: 1, codex: 1 },
          fullHistory: false,
        },
      ),
    );

    expect(disappeared(json.warnings, "codex")).toHaveLength(1);
    expect(activeTimeNotices(json.warnings)).toHaveLength(1);
    expect(storedDay(store, "2025-12-02")!.sourceBreakdown.codex.tokens).toBe(500);
  });

  it("leaves full-history submits unchanged", async () => {
    const store = newStore();
    seedDay(store, "2026-01-15", { codex: cell(500, 0.5) }, 3_600_000);

    const json = await submit(
      store,
      submissionBody(
        [{ date: "2026-01-15", activeTimeMs: 600_000, clients: [{ client: "droid", tokens: 100 }] }],
        {
          clients: ["droid", "codex"],
          parserVersions: { droid: 1, codex: 1 },
          fullHistory: true,
          fullHistoryClients: ["droid", "codex"],
        },
      ),
    );

    // A full snapshot genuinely covers everything, so codex's absence is the
    // real signal the notice exists for.
    expect(disappeared(json.warnings, "codex")).toHaveLength(1);
    expect(activeTimeNotices(json.warnings)).toHaveLength(1);
    expect(storedDay(store, "2026-01-15")!.sourceBreakdown.codex.tokens).toBe(500);
  });
});
