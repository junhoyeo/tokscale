import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

function createLegacyClientBreakdown(overrides: Partial<{
  tokens: number;
  cost: number;
  input: number;
  output: number;
  cacheRead: number;
  cacheWrite: number;
  reasoning: number;
  messages: number;
  models: Record<string, { tokens: number; cost: number }>;
  modelId: string;
}> = {}) {
  return {
    tokens: overrides.tokens ?? 150,
    cost: overrides.cost ?? 1.5,
    input: overrides.input ?? 100,
    output: overrides.output ?? 50,
    cacheRead: overrides.cacheRead ?? 0,
    cacheWrite: overrides.cacheWrite ?? 0,
    reasoning: overrides.reasoning ?? 0,
    messages: overrides.messages ?? 2,
    models: overrides.models ?? {
      "claude-sonnet-4": {
        tokens: overrides.tokens ?? 150,
        cost: overrides.cost ?? 1.5,
      },
    },
    modelId: overrides.modelId,
  };
}

function createDailyRow(overrides: Partial<{
  date: string;
  tokens: number;
  cost: string;
  timestampMs: number | null;
  sourceBreakdown: Record<string, unknown> | null;
}> = {}) {
  return {
    date: overrides.date ?? "2026-03-10",
    tokens: overrides.tokens ?? 150,
    cost: overrides.cost ?? "1.5000",
    timestampMs: overrides.timestampMs ?? null,
    sourceBreakdown: overrides.sourceBreakdown ?? null,
  };
}

function createMockState() {
  const selectResults: Array<Array<Record<string, unknown>>> = [];
  const authenticatePersonalToken = vi.fn();

  const tables = {
    submissions: {
      id: "submissions.id",
      userId: "submissions.userId",
    },
    dailyBreakdown: {
      submissionId: "dailyBreakdown.submissionId",
      date: "dailyBreakdown.date",
      tokens: "dailyBreakdown.tokens",
      cost: "dailyBreakdown.cost",
      timestampMs: "dailyBreakdown.timestampMs",
      sourceBreakdown: "dailyBreakdown.sourceBreakdown",
    },
  };

  const eq = vi.fn(() => "eq");

  function nextSelectResult<T>(): T[] {
    return (selectResults.shift() ?? []) as T[];
  }

  const db = {
    select: vi.fn(() => {
      const builder = {
        from: vi.fn(() => builder),
        where: vi.fn(() => builder),
        orderBy: vi.fn(() => builder),
        limit: vi.fn(async () => nextSelectResult()),
        then: (resolve: (value: unknown) => unknown) => resolve(nextSelectResult()),
      };

      return builder;
    }),
  };

  return {
    authenticatePersonalToken,
    db,
    tables,
    eq,
    reset() {
      selectResults.length = 0;
      authenticatePersonalToken.mockReset();
      db.select.mockClear();
      eq.mockClear();
    },
    pushSelectResult(rows: Array<Record<string, unknown>>) {
      selectResults.push(rows);
    },
  };
}

function getMockState() {
  const globalForMocks = globalThis as typeof globalThis & {
    __meStatsMockState?: ReturnType<typeof createMockState>;
  };

  if (!globalForMocks.__meStatsMockState) {
    globalForMocks.__meStatsMockState = createMockState();
  }

  return globalForMocks.__meStatsMockState;
}

const mockState = getMockState();

vi.mock("@/lib/auth/personalTokens", () => ({
  authenticatePersonalToken: getMockState().authenticatePersonalToken,
}));

vi.mock("@/lib/db", () => ({
  db: getMockState().db,
  submissions: getMockState().tables.submissions,
  dailyBreakdown: getMockState().tables.dailyBreakdown,
}));

vi.mock("drizzle-orm", () => ({
  eq: getMockState().eq,
}));

type ModuleExports = typeof import("../../src/app/api/me/stats/route");

let GET: ModuleExports["GET"];

beforeAll(async () => {
  const routeModule = await import("../../src/app/api/me/stats/route");
  GET = routeModule.GET;
});

beforeEach(() => {
  mockState.reset();
});

describe("GET /api/me/stats", () => {
  it("returns 401 when the bearer token is missing", async () => {
    const response = await GET(new Request("http://localhost:3000/api/me/stats"));

    expect(response.status).toBe(401);
    expect(mockState.authenticatePersonalToken).not.toHaveBeenCalled();
    expect(await response.json()).toEqual({
      error: "Missing or invalid Authorization header",
    });
  });

  it("returns empty stats when the authenticated user has no submission", async () => {
    mockState.authenticatePersonalToken.mockResolvedValue({
      status: "valid",
      tokenId: "token-1",
      userId: "user-1",
      username: "alice",
      displayName: "Alice",
      avatarUrl: null,
      isAdmin: false,
      expiresAt: null,
    });
    mockState.pushSelectResult([]);

    const response = await GET(
      new Request("http://localhost:3000/api/me/stats", {
        headers: {
          Authorization: "Bearer tt_valid",
        },
      })
    );
    const body = await response.json();

    expect(response.status).toBe(200);
    expect(mockState.authenticatePersonalToken).toHaveBeenCalledWith("tt_valid", {
      touchLastUsedAt: false,
    });
    expect(body).toEqual({
      totalCost: 0,
      totalTokens: 0,
      byModel: [],
      byDay: [],
      byClient: [],
      devices: [],
    });
  });

  it("returns aggregated stats for legacy source breakdown data", async () => {
    mockState.authenticatePersonalToken.mockResolvedValue({
      status: "valid",
      tokenId: "token-1",
      userId: "user-1",
      username: "alice",
      displayName: "Alice",
      avatarUrl: null,
      isAdmin: false,
      expiresAt: null,
    });
    mockState.pushSelectResult([{ id: "submission-1" }]);
    mockState.pushSelectResult([
      createDailyRow({
        date: "2026-03-10",
        tokens: 150,
        cost: "1.5000",
        sourceBreakdown: {
          claude: createLegacyClientBreakdown({
            tokens: 150,
            cost: 1.5,
            models: {
              "claude-sonnet-4": { tokens: 150, cost: 1.5 },
            },
          }),
        },
      }),
      createDailyRow({
        date: "2026-03-11",
        tokens: 80,
        cost: "0.8000",
        sourceBreakdown: {
          cursor: createLegacyClientBreakdown({
            tokens: 80,
            cost: 0.8,
            models: {
              "gpt-4.1": { tokens: 80, cost: 0.8 },
            },
          }),
        },
      }),
    ]);

    const response = await GET(
      new Request("http://localhost:3000/api/me/stats", {
        headers: {
          Authorization: "Bearer tt_valid",
        },
      })
    );
    const body = await response.json();

    expect(response.status).toBe(200);
    expect(body).toEqual({
      totalCost: 2.3,
      totalTokens: 230,
      byModel: [
        { model: "claude-sonnet-4", cost: 1.5, tokens: 150 },
        { model: "gpt-4.1", cost: 0.8, tokens: 80 },
      ],
      byDay: [
        { date: "2026-03-10", cost: 1.5, tokens: 150 },
        { date: "2026-03-11", cost: 0.8, tokens: 80 },
      ],
      byClient: [
        { client: "claude", cost: 1.5, tokens: 150 },
        { client: "cursor", cost: 0.8, tokens: 80 },
      ],
      devices: [],
    });
  });

  it("aggregates usage across devices when device breakdown data exists", async () => {
    mockState.authenticatePersonalToken.mockResolvedValue({
      status: "valid",
      tokenId: "token-1",
      userId: "user-1",
      username: "alice",
      displayName: "Alice",
      avatarUrl: null,
      isAdmin: false,
      expiresAt: null,
    });
    mockState.pushSelectResult([{ id: "submission-1" }]);
    mockState.pushSelectResult([
      createDailyRow({
        date: "2026-03-10",
        tokens: 260,
        cost: "2.6000",
        timestampMs: Date.parse("2026-03-10T08:00:00.000Z"),
        sourceBreakdown: {
          devices: {
            laptop: {
              claude: createLegacyClientBreakdown({
                tokens: 150,
                cost: 1.5,
                models: {
                  "claude-sonnet-4": { tokens: 150, cost: 1.5 },
                },
              }),
            },
            iphone: {
              cursor: createLegacyClientBreakdown({
                tokens: 110,
                cost: 1.1,
                models: {
                  "gpt-4.1": { tokens: 110, cost: 1.1 },
                },
              }),
            },
          },
        },
      }),
      createDailyRow({
        date: "2026-03-11",
        tokens: 90,
        cost: "0.9000",
        timestampMs: Date.parse("2026-03-11T12:30:00.000Z"),
        sourceBreakdown: {
          devices: {
            laptop: {
              claude: createLegacyClientBreakdown({
                tokens: 40,
                cost: 0.4,
                models: {
                  "claude-sonnet-4": { tokens: 40, cost: 0.4 },
                },
              }),
            },
            ipad: {
              claude: createLegacyClientBreakdown({
                tokens: 50,
                cost: 0.5,
                models: {
                  "claude-opus-4": { tokens: 50, cost: 0.5 },
                },
              }),
            },
          },
        },
      }),
    ]);

    const response = await GET(
      new Request("http://localhost:3000/api/me/stats", {
        headers: {
          Authorization: "Bearer tt_valid",
        },
      })
    );
    const body = await response.json();

    expect(response.status).toBe(200);
    expect(body).toEqual({
      totalCost: 3.5,
      totalTokens: 350,
      byModel: [
        { model: "claude-sonnet-4", cost: 1.9, tokens: 190 },
        { model: "gpt-4.1", cost: 1.1, tokens: 110 },
        { model: "claude-opus-4", cost: 0.5, tokens: 50 },
      ],
      byDay: [
        { date: "2026-03-10", cost: 2.6, tokens: 260 },
        { date: "2026-03-11", cost: 0.9, tokens: 90 },
      ],
      byClient: [
        { client: "claude", cost: 2.4, tokens: 240 },
        { client: "cursor", cost: 1.1, tokens: 110 },
      ],
      devices: [
        {
          id: "laptop",
          lastSeenAt: "2026-03-11T12:30:00.000Z",
          cost: 1.9,
        },
        {
          id: "iphone",
          lastSeenAt: "2026-03-10T08:00:00.000Z",
          cost: 1.1,
        },
        {
          id: "ipad",
          lastSeenAt: "2026-03-11T12:30:00.000Z",
          cost: 0.5,
        },
      ],
    });
  });
});
