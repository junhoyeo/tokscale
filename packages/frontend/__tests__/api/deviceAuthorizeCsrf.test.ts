import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

const mockState = vi.hoisted(() => {
  const getSession = vi.fn();
  const getSessionFromHeader = vi.fn();
  const eq = vi.fn((left: unknown, right: unknown) => ({ op: "eq", left, right }));
  const gt = vi.fn((left: unknown, right: unknown) => ({ op: "gt", left, right }));
  const isNull = vi.fn((value: unknown) => ({ op: "isNull", value }));
  const and = vi.fn((...conditions: unknown[]) => ({ op: "and", conditions }));
  const limit = vi.fn(async () => records);
  const whereSelect = vi.fn(() => ({ limit }));
  const from = vi.fn(() => ({ where: whereSelect }));
  const select = vi.fn(() => ({ from }));
  const whereUpdate = vi.fn(async () => undefined);
  const set = vi.fn(() => ({ where: whereUpdate }));
  const update = vi.fn(() => ({ set }));
  let records: Array<{ id: string }> = [];

  const deviceCodes = {
    id: "deviceCodes.id",
    userCode: "deviceCodes.userCode",
    expiresAt: "deviceCodes.expiresAt",
    userId: "deviceCodes.userId",
  };

  return {
    getSession,
    getSessionFromHeader,
    eq,
    gt,
    isNull,
    and,
    db: { select, update },
    deviceCodes,
    setRecords(value: Array<{ id: string }>) {
      records = value;
    },
    reset() {
      getSession.mockReset();
      getSessionFromHeader.mockReset();
      eq.mockClear();
      gt.mockClear();
      isNull.mockClear();
      and.mockClear();
      limit.mockClear();
      whereSelect.mockClear();
      from.mockClear();
      select.mockClear();
      whereUpdate.mockClear();
      set.mockClear();
      update.mockClear();
      records = [];
    },
  };
});

vi.mock("drizzle-orm", () => ({
  eq: mockState.eq,
  gt: mockState.gt,
  isNull: mockState.isNull,
  and: mockState.and,
}));

vi.mock("@/lib/auth/session", () => ({
  getSession: mockState.getSession,
  getSessionFromHeader: mockState.getSessionFromHeader,
}));

vi.mock("@/lib/db", () => ({
  db: mockState.db,
  deviceCodes: mockState.deviceCodes,
}));

type ModuleExports = typeof import("../../src/app/api/auth/device/authorize/route");

let POST: ModuleExports["POST"];

beforeAll(async () => {
  const routeModule = await import("../../src/app/api/auth/device/authorize/route");
  POST = routeModule.POST;
});

beforeEach(() => {
  mockState.reset();
});

function session() {
  return {
    id: "user-1",
    username: "alice",
    displayName: "Alice",
    avatarUrl: null,
  };
}

function createRequest(headers: Record<string, string> = { Origin: "http://localhost:3000" }) {
  return new Request("http://localhost:3000/api/auth/device/authorize", {
    method: "POST",
    headers,
    body: JSON.stringify({ userCode: "ABCD-1234" }),
  });
}

describe("POST /api/auth/device/authorize CSRF origin checks", () => {
  it("returns 401 for cookie auth when Origin is missing", async () => {
    mockState.getSession.mockResolvedValue(session());
    mockState.setRecords([{ id: "device-code-1" }]);

    const response = await POST(createRequest({}));

    expect(response.status).toBe(401);
    expect(await response.json()).toEqual({ error: "Not authenticated" });
    expect(mockState.getSession).not.toHaveBeenCalled();
    expect(mockState.db.update).not.toHaveBeenCalled();
  });

  it("returns 401 for cookie auth when Origin is not allowed", async () => {
    mockState.getSession.mockResolvedValue(session());
    mockState.setRecords([{ id: "device-code-1" }]);

    const response = await POST(
      createRequest({ Origin: "https://attacker.example" })
    );

    expect(response.status).toBe(401);
    expect(await response.json()).toEqual({ error: "Not authenticated" });
    expect(mockState.getSession).not.toHaveBeenCalled();
    expect(mockState.db.update).not.toHaveBeenCalled();
  });

  it("authorizes device code when cookie Origin is allowed", async () => {
    mockState.getSession.mockResolvedValue(session());
    mockState.setRecords([{ id: "device-code-1" }]);

    const response = await POST(createRequest());

    expect(response.status).toBe(200);
    expect(await response.json()).toEqual({ success: true });
    expect(mockState.db.update).toHaveBeenCalledWith(mockState.deviceCodes);
  });
});
