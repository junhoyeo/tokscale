import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

const mockState = vi.hoisted(() => {
  const selectResults: Array<Array<Record<string, unknown>>> = [];
  const deleteResults: Array<Array<Record<string, unknown>>> = [];
  const forUpdateCalls: string[] = [];

  const tables = {
    deviceCodes: {
      id: "deviceCodes.id",
      deviceCode: "deviceCodes.deviceCode",
      expiresAt: "deviceCodes.expiresAt",
      userId: "deviceCodes.userId",
    },
    users: {
      id: "users.id",
    },
  };

  const eq = vi.fn(() => "eq");
  const and = vi.fn(() => "and");
  const gt = vi.fn(() => "gt");

  function nextResult<T>(queue: T[][]): T[] {
    return queue.shift() ?? [];
  }

  const db = {
    select: vi.fn(() => {
      const builder = {
        from: vi.fn(() => builder),
        where: vi.fn(() => builder),
        for: vi.fn((mode: string) => {
          forUpdateCalls.push(mode);
          return builder;
        }),
        limit: vi.fn(async () => nextResult(selectResults)),
      };

      return builder;
    }),
    delete: vi.fn(() => {
      const builder = {
        where: vi.fn(async () => nextResult(deleteResults)),
      };

      return builder;
    }),
    transaction: vi.fn(async (callback: (tx: unknown) => Promise<unknown>) => callback(tx)),
  };

  const tx = {
    select: db.select,
    delete: db.delete,
  };

  return {
    db,
    tables,
    eq,
    and,
    gt,
    forUpdateCalls,
    reset() {
      selectResults.length = 0;
      deleteResults.length = 0;
      forUpdateCalls.length = 0;
      db.select.mockClear();
      db.delete.mockClear();
      db.transaction.mockClear();
      eq.mockClear();
      and.mockClear();
      gt.mockClear();
    },
    pushSelectResult(rows: Array<Record<string, unknown>>) {
      selectResults.push(rows);
    },
    pushDeleteResult(rows: Array<Record<string, unknown>> = []) {
      deleteResults.push(rows);
    },
  };
});

const issuePersonalTokenInTransaction = vi.fn();

vi.mock("@/lib/db", () => ({
  db: mockState.db,
  deviceCodes: mockState.tables.deviceCodes,
  users: mockState.tables.users,
}));

vi.mock("drizzle-orm", () => ({
  eq: mockState.eq,
  and: mockState.and,
  gt: mockState.gt,
}));

vi.mock("@/lib/auth/personalTokens", () => ({
  issuePersonalTokenInTransaction,
}));

type ModuleExports = typeof import("../../src/app/api/auth/device/poll/route");

let POST: ModuleExports["POST"];

beforeAll(async () => {
  const routeModule = await import("../../src/app/api/auth/device/poll/route");
  POST = routeModule.POST;
});

beforeEach(() => {
  mockState.reset();
  issuePersonalTokenInTransaction.mockReset();
});

describe("POST /api/auth/device/poll", () => {
  it("locks the device code row and issues the token inside the transaction", async () => {
    mockState.pushSelectResult([
      {
        id: "device-1",
        userId: "user-1",
        deviceName: "CLI on macbook",
        expiresAt: new Date("2026-03-08T05:00:00.000Z"),
      },
    ]);
    mockState.pushSelectResult([
      {
        id: "user-1",
        username: "alice",
        avatarUrl: "https://example.com/alice.png",
      },
    ]);
    mockState.pushDeleteResult();
    issuePersonalTokenInTransaction.mockResolvedValue({
      id: "token-1",
      userId: "user-1",
      name: "CLI on macbook",
      token: "tt_test_token",
      createdAt: new Date("2026-03-08T04:00:00.000Z"),
      lastUsedAt: null,
      expiresAt: null,
    });

    const response = await POST(
      new Request("http://localhost:3000/api/auth/device/poll", {
        method: "POST",
        body: JSON.stringify({ deviceCode: "device-code-1" }),
      })
    );
    const body = await response.json();

    expect(response.status).toBe(200);
    expect(mockState.db.transaction).toHaveBeenCalledTimes(1);
    expect(mockState.forUpdateCalls).toEqual(["update"]);
    expect(issuePersonalTokenInTransaction).toHaveBeenCalledTimes(1);
    expect(issuePersonalTokenInTransaction).toHaveBeenNthCalledWith(1, expect.anything(), {
      userId: "user-1",
      name: "CLI on macbook",
      ensureUniqueName: true,
    });
    expect(mockState.db.delete).toHaveBeenCalledTimes(1);
    expect(mockState.eq).toHaveBeenNthCalledWith(3, mockState.tables.deviceCodes.id, "device-1");
    expect(body).toEqual({
      status: "complete",
      token: "tt_test_token",
      user: {
        username: "alice",
        avatarUrl: "https://example.com/alice.png",
      },
    });
  });

  it("returns 400 when deviceCode is missing", async () => {
    const response = await POST(
      new Request("http://localhost:3000/api/auth/device/poll", {
        method: "POST",
        body: JSON.stringify({}),
      })
    );
    const body = await response.json();

    expect(response.status).toBe(400);
    expect(body).toEqual({ error: "Missing device code" });
  });

  it("returns expired status when no matching device code found", async () => {
    mockState.pushSelectResult([]);

    const response = await POST(
      new Request("http://localhost:3000/api/auth/device/poll", {
        method: "POST",
        body: JSON.stringify({ deviceCode: "invalid-code" }),
      })
    );
    const body = await response.json();

    expect(response.status).toBe(200);
    expect(body).toEqual({ status: "expired" });
    expect(mockState.forUpdateCalls).toEqual(["update"]);
    expect(issuePersonalTokenInTransaction).not.toHaveBeenCalled();
  });

  it("returns pending status when user has not yet authorized", async () => {
    mockState.pushSelectResult([
      {
        id: "device-1",
        userId: null,
        deviceName: "CLI on macbook",
        expiresAt: new Date("2026-03-08T05:00:00.000Z"),
      },
    ]);

    const response = await POST(
      new Request("http://localhost:3000/api/auth/device/poll", {
        method: "POST",
        body: JSON.stringify({ deviceCode: "device-code-1" }),
      })
    );
    const body = await response.json();

    expect(response.status).toBe(200);
    expect(body).toEqual({ status: "pending" });
    expect(mockState.forUpdateCalls).toEqual(["update"]);
    expect(issuePersonalTokenInTransaction).not.toHaveBeenCalled();
  });

  it("returns 500 when authorized user is not found in users table", async () => {
    mockState.pushSelectResult([
      {
        id: "device-1",
        userId: "user-1",
        deviceName: "CLI on macbook",
        expiresAt: new Date("2026-03-08T05:00:00.000Z"),
      },
    ]);
    mockState.pushSelectResult([]);

    const response = await POST(
      new Request("http://localhost:3000/api/auth/device/poll", {
        method: "POST",
        body: JSON.stringify({ deviceCode: "device-code-1" }),
      })
    );
    const body = await response.json();

    expect(response.status).toBe(500);
    expect(body).toEqual({ error: "User not found" });
  });
});
