import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

const mockState = vi.hoisted(() => {
  const authenticatePersonalToken = vi.fn();
  const awaitedResults: unknown[] = [];

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
      inputTokens: "dailyBreakdown.inputTokens",
      outputTokens: "dailyBreakdown.outputTokens",
    },
    submittedDevices: {
      id: "submittedDevices.id",
      userId: "submittedDevices.userId",
      deviceKey: "submittedDevices.deviceKey",
      displayName: "submittedDevices.displayName",
      lastSubmittedAt: "submittedDevices.lastSubmittedAt",
    },
  };

  const eq = vi.fn(() => "eq");
  const asc = vi.fn(() => "asc");
  const desc = vi.fn(() => "desc");
  const sql = Object.assign(
    vi.fn((strings: TemplateStringsArray, ...values: unknown[]) => ({
      strings: Array.from(strings),
      values,
    })),
    { raw: vi.fn() }
  );

  const db = {
    select: vi.fn(() => {
      const builder = {
        from: vi.fn(() => builder),
        where: vi.fn(() => builder),
        groupBy: vi.fn(() => builder),
        orderBy: vi.fn(() => builder),
        limit: vi.fn(() => builder),
        then: (
          resolve: (value: unknown) => unknown,
          reject?: (reason: unknown) => unknown
        ) => {
          const next = awaitedResults.shift() ?? [];
          if (next instanceof Error) {
            return reject ? reject(next) : Promise.reject(next);
          }
          return resolve(next);
        },
      };
      return builder;
    }),
  };

  return {
    authenticatePersonalToken,
    tables,
    eq,
    asc,
    desc,
    sql,
    db,
    pushResult(value: unknown) {
      awaitedResults.push(value);
    },
    reset() {
      authenticatePersonalToken.mockReset();
      awaitedResults.length = 0;
      db.select.mockClear();
      eq.mockClear();
      asc.mockClear();
      desc.mockClear();
      sql.mockClear();
    },
  };
});

vi.mock("@/lib/auth/personalTokens", () => ({
  authenticatePersonalToken: mockState.authenticatePersonalToken,
}));

vi.mock("@/lib/db", () => ({
  db: mockState.db,
  submissions: mockState.tables.submissions,
  dailyBreakdown: mockState.tables.dailyBreakdown,
  submittedDevices: mockState.tables.submittedDevices,
}));

vi.mock("drizzle-orm", () => ({
  eq: mockState.eq,
  asc: mockState.asc,
  desc: mockState.desc,
  sql: mockState.sql,
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

function createRequest(headers: Record<string, string> = {}) {
  return new Request("http://localhost:3000/api/me/stats", { headers });
}

const VALID_AUTH = {
  status: "valid" as const,
  tokenId: "token-1",
  userId: "user-1",
  username: "alice",
  displayName: "Alice",
  avatarUrl: null,
  expiresAt: null,
};

describe("GET /api/me/stats", () => {
  it("returns 401 when the bearer token is missing", async () => {
    const response = await GET(createRequest());

    expect(response.status).toBe(401);
    expect(await response.json()).toEqual({
      error: "Missing or invalid Authorization header",
    });
    expect(mockState.authenticatePersonalToken).not.toHaveBeenCalled();
  });

  it("does not accept cookie-based sessions", async () => {
    const response = await GET(
      createRequest({ Cookie: "session=browser-session-token" })
    );

    expect(response.status).toBe(401);
    expect(mockState.authenticatePersonalToken).not.toHaveBeenCalled();
  });

  it("returns 401 when the bearer token is invalid", async () => {
    mockState.authenticatePersonalToken.mockResolvedValue({ status: "invalid" });

    const response = await GET(
      createRequest({ Authorization: "Bearer tt_invalid" })
    );

    expect(response.status).toBe(401);
    expect(mockState.authenticatePersonalToken).toHaveBeenCalledWith(
      "tt_invalid",
      { touchLastUsedAt: false }
    );
    expect(await response.json()).toEqual({ error: "Invalid API token" });
  });

  it("returns 401 when the bearer token has expired", async () => {
    mockState.authenticatePersonalToken.mockResolvedValue({ status: "expired" });

    const response = await GET(
      createRequest({ Authorization: "Bearer tt_expired" })
    );

    expect(response.status).toBe(401);
    expect(await response.json()).toEqual({ error: "API token has expired" });
  });

  it("returns empty stats when the user has no submission", async () => {
    mockState.authenticatePersonalToken.mockResolvedValue(VALID_AUTH);
    mockState.pushResult([]); // submissions lookup

    const response = await GET(
      createRequest({ Authorization: "Bearer tt_valid" })
    );

    expect(response.status).toBe(200);
    expect(await response.json()).toEqual({
      schemaVersion: 1,
      totalTokens: 0,
      totalCost: 0,
      deviceCount: 0,
      lastSubmittedAt: null,
      days: [],
      devices: [],
    });
    // Only the submission lookup should have run.
    expect(mockState.db.select).toHaveBeenCalledTimes(1);
  });

  it("aggregates per-day totals across devices and reports device metadata", async () => {
    mockState.authenticatePersonalToken.mockResolvedValue(VALID_AUTH);
    mockState.pushResult([{ id: "submission-1" }]);
    // Day rows are GROUP BY date sums across submitted_device_id; SQL SUM()
    // comes back as strings from the driver.
    mockState.pushResult([
      {
        date: "2026-06-01",
        tokens: "1000",
        inputTokens: "600",
        outputTokens: "400",
        cost: "1.5",
      },
      {
        date: "2026-06-02",
        tokens: "250",
        inputTokens: "150",
        outputTokens: "100",
        cost: "0.25",
      },
    ]);
    mockState.pushResult([
      {
        id: "device-2",
        deviceKey: "dev_abc",
        displayName: "Work laptop",
        lastSubmittedAt: new Date("2026-06-02T12:00:00.000Z"),
      },
      {
        id: "device-1",
        deviceKey: "legacy-default",
        displayName: null,
        lastSubmittedAt: new Date("2026-06-01T08:00:00.000Z"),
      },
    ]);

    const response = await GET(
      createRequest({ Authorization: "Bearer tt_valid" })
    );

    expect(response.status).toBe(200);
    expect(await response.json()).toEqual({
      schemaVersion: 1,
      totalTokens: 1250,
      totalCost: 1.75,
      deviceCount: 2,
      lastSubmittedAt: "2026-06-02T12:00:00.000Z",
      days: [
        {
          date: "2026-06-01",
          tokens: 1000,
          inputTokens: 600,
          outputTokens: 400,
          cost: 1.5,
        },
        {
          date: "2026-06-02",
          tokens: 250,
          inputTokens: 150,
          outputTokens: 100,
          cost: 0.25,
        },
      ],
      devices: [
        {
          id: "device-2",
          displayName: "Work laptop",
          lastSubmittedAt: "2026-06-02T12:00:00.000Z",
        },
        {
          id: "device-1",
          displayName: "Legacy submissions",
          lastSubmittedAt: "2026-06-01T08:00:00.000Z",
        },
      ],
    });
  });

  it("handles devices that have never submitted", async () => {
    mockState.authenticatePersonalToken.mockResolvedValue(VALID_AUTH);
    mockState.pushResult([{ id: "submission-1" }]);
    mockState.pushResult([]); // no daily rows
    mockState.pushResult([
      {
        id: "device-1",
        deviceKey: "dev_new",
        displayName: null,
        lastSubmittedAt: null,
      },
    ]);

    const response = await GET(
      createRequest({ Authorization: "Bearer tt_valid" })
    );

    expect(response.status).toBe(200);
    expect(await response.json()).toEqual({
      schemaVersion: 1,
      totalTokens: 0,
      totalCost: 0,
      deviceCount: 1,
      lastSubmittedAt: null,
      days: [],
      devices: [
        {
          id: "device-1",
          displayName: "Unnamed device",
          lastSubmittedAt: null,
        },
      ],
    });
  });

  it("returns 500 when the database query fails", async () => {
    mockState.authenticatePersonalToken.mockResolvedValue(VALID_AUTH);
    mockState.pushResult(new Error("connection refused"));

    const response = await GET(
      createRequest({ Authorization: "Bearer tt_valid" })
    );

    expect(response.status).toBe(500);
    expect(await response.json()).toEqual({ error: "Internal server error" });
  });
});
