import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

// GET /api/users/[username]/devices response-shape tests. The settings page
// edits the raw `customName` while public profiles render the resolved
// `displayName`, so both must be present and must not be conflated.

interface DeviceRow {
  id: string;
  deviceKey: string;
  displayName: string | null;
  createdAt: Date | null;
  lastSubmittedAt: Date | null;
  totalTokens: string;
  totalCost: string;
  inputTokens: string;
  outputTokens: string;
  activeDays: string;
  firstDay: string | null;
  lastDay: string | null;
}

const mockState = vi.hoisted(() => {
  let userRows: unknown[] = [];
  let deviceRows: unknown[] = [];

  // The route issues two select chains: the username lookup
  // (.from().where().limit()) and the device aggregation
  // (.from().leftJoin().where().groupBy().orderBy()).
  const select = vi.fn(() => ({
    from: () => ({
      where: () => ({
        limit: async () => userRows,
      }),
      leftJoin: () => ({
        where: () => ({
          groupBy: () => ({
            orderBy: async () => deviceRows,
          }),
        }),
      }),
    }),
  }));

  return {
    select,
    setUserRows(rows: unknown[]) {
      userRows = rows;
    },
    setDeviceRows(rows: DeviceRow[]) {
      deviceRows = rows;
    },
    reset() {
      select.mockClear();
      userRows = [];
      deviceRows = [];
    },
  };
});

vi.mock("@/lib/db", () => ({
  db: { select: mockState.select },
  users: {
    id: "users.id",
    username: "users.username",
    displayName: "users.displayName",
    avatarUrl: "users.avatarUrl",
  },
  submittedDevices: {
    id: "submittedDevices.id",
    deviceKey: "submittedDevices.deviceKey",
    displayName: "submittedDevices.displayName",
    userId: "submittedDevices.userId",
    createdAt: "submittedDevices.createdAt",
    lastSubmittedAt: "submittedDevices.lastSubmittedAt",
  },
  dailyBreakdown: {
    tokens: "dailyBreakdown.tokens",
    cost: "dailyBreakdown.cost",
    inputTokens: "dailyBreakdown.inputTokens",
    outputTokens: "dailyBreakdown.outputTokens",
    date: "dailyBreakdown.date",
    submittedDeviceId: "dailyBreakdown.submittedDeviceId",
  },
}));

vi.mock("drizzle-orm", () => ({
  sql: Object.assign(
    (strings: TemplateStringsArray, ...values: unknown[]) => ({
      strings: Array.from(strings),
      values,
    }),
    { raw: (value: string) => value }
  ),
  desc: (column: unknown) => ({ op: "desc", column }),
  eq: (left: unknown, right: unknown) => ({ op: "eq", left, right }),
}));

// Mocked (matching usersProfile.test.ts) so the route does not pull in the
// real drizzle schema through usernameLookup's `./schema` import.
vi.mock("@/lib/db/usernameLookup", () => ({
  USERNAME_LOOKUP_LIMIT: 2,
  usernameEqualsIgnoreCase: (username: string) => ({
    op: "usernameEqualsIgnoreCase",
    username,
  }),
  getSingleUsernameMatch: (rows: unknown[]) => {
    if (rows.length > 1) {
      throw new Error("ambiguous username");
    }
    return rows[0] ?? null;
  },
}));

type ModuleExports = typeof import("../../src/app/api/users/[username]/devices/route");

let GET: ModuleExports["GET"];

beforeAll(async () => {
  const routeModule = await import(
    "../../src/app/api/users/[username]/devices/route"
  );
  GET = routeModule.GET;
});

beforeEach(() => {
  mockState.reset();
});

function deviceRow(overrides: Partial<DeviceRow> = {}): DeviceRow {
  return {
    id: "11111111-1111-4111-8111-111111111111",
    deviceKey: "machine-aaaa",
    displayName: null,
    createdAt: new Date("2026-06-01T00:00:00.000Z"),
    lastSubmittedAt: new Date("2026-06-09T12:00:00.000Z"),
    totalTokens: "1000",
    totalCost: "12.5",
    inputTokens: "600",
    outputTokens: "400",
    activeDays: "3",
    firstDay: "2026-06-01",
    lastDay: "2026-06-09",
    ...overrides,
  };
}

function request(): Request {
  return new Request("https://tokscale.ai/api/users/alice/devices");
}

const params = { params: Promise.resolve({ username: "alice" }) };

describe("GET /api/users/[username]/devices", () => {
  it("returns 404 when the user does not exist", async () => {
    mockState.setUserRows([]);

    const response = await GET(request(), params);

    expect(response.status).toBe(404);
    expect(await response.json()).toEqual({ error: "User not found" });
  });

  it("exposes both the resolved displayName and the raw customName", async () => {
    mockState.setUserRows([
      { id: "u1", username: "alice", displayName: "Alice", avatarUrl: null },
    ]);
    mockState.setDeviceRows([
      deviceRow({ id: "d1", deviceKey: "machine-aaaa", displayName: null }),
      deviceRow({ id: "d2", deviceKey: "machine-bbbb", displayName: "Work laptop" }),
      // A custom name that happens to equal the fallback label must round-trip
      // as a real customName, not be treated as "never renamed".
      deviceRow({ id: "d3", deviceKey: "machine-cccc", displayName: "Unnamed device" }),
    ]);

    const response = await GET(request(), params);
    expect(response.status).toBe(200);

    const body = await response.json();
    expect(body.user).toEqual({
      username: "alice",
      displayName: "Alice",
      avatarUrl: null,
    });

    const [unnamed, named, fallbackNamed] = body.devices;

    expect(unnamed).toMatchObject({
      id: "d1",
      displayName: "Unnamed device",
      customName: null,
    });
    expect(named).toMatchObject({
      id: "d2",
      displayName: "Work laptop",
      customName: "Work laptop",
    });
    expect(fallbackNamed).toMatchObject({
      id: "d3",
      displayName: "Unnamed device",
      customName: "Unnamed device",
    });
  });

  it("coerces SQL aggregate strings to numbers and timestamps to ISO strings", async () => {
    mockState.setUserRows([
      { id: "u1", username: "alice", displayName: null, avatarUrl: null },
    ]);
    mockState.setDeviceRows([deviceRow()]);

    const response = await GET(request(), params);
    const body = await response.json();

    expect(body.devices[0]).toMatchObject({
      totalTokens: 1000,
      totalCost: 12.5,
      inputTokens: 600,
      outputTokens: 400,
      activeDays: 3,
      createdAt: "2026-06-01T00:00:00.000Z",
      lastSubmittedAt: "2026-06-09T12:00:00.000Z",
      firstDay: "2026-06-01",
      lastDay: "2026-06-09",
    });
  });
});
