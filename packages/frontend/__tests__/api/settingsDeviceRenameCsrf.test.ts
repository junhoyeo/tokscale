import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

const mockState = vi.hoisted(() => {
  const getSession = vi.fn();
  const getSessionFromHeader = vi.fn();
  const revalidateTag = vi.fn();
  const revalidateUsernamePaths = vi.fn();
  const eq = vi.fn((left: unknown, right: unknown) => ({ op: "eq", left, right }));
  const and = vi.fn((...conditions: unknown[]) => ({ op: "and", conditions }));
  const returning = vi.fn(async () => updatedRows);
  const where = vi.fn(() => ({ returning }));
  const set = vi.fn(() => ({ where }));
  const update = vi.fn(() => ({ set }));
  let updatedRows: Array<{ id: string; deviceKey: string; displayName: string | null }> = [];

  const submittedDevices = {
    id: "submittedDevices.id",
    deviceKey: "submittedDevices.deviceKey",
    displayName: "submittedDevices.displayName",
    userId: "submittedDevices.userId",
    updatedAt: "submittedDevices.updatedAt",
  };

  return {
    getSession,
    getSessionFromHeader,
    revalidateTag,
    revalidateUsernamePaths,
    eq,
    and,
    db: { update },
    submittedDevices,
    setUpdatedRows(rows: Array<{ id: string; deviceKey: string; displayName: string | null }>) {
      updatedRows = rows;
    },
    reset() {
      getSession.mockReset();
      getSessionFromHeader.mockReset();
      revalidateTag.mockReset();
      revalidateUsernamePaths.mockReset();
      eq.mockClear();
      and.mockClear();
      returning.mockClear();
      where.mockClear();
      set.mockClear();
      update.mockClear();
      updatedRows = [];
    },
  };
});

vi.mock("next/cache", () => ({
  revalidateTag: mockState.revalidateTag,
}));

vi.mock("drizzle-orm", () => ({
  eq: mockState.eq,
  and: mockState.and,
}));

vi.mock("@/lib/auth/session", () => ({
  getSession: mockState.getSession,
  getSessionFromHeader: mockState.getSessionFromHeader,
}));

vi.mock("@/lib/db", () => ({
  db: mockState.db,
  submittedDevices: mockState.submittedDevices,
}));

vi.mock("@/lib/db/usernameLookup", () => ({
  normalizeUsernameCacheKey: (username: string) => username.toLowerCase(),
  revalidateUsernamePaths: mockState.revalidateUsernamePaths,
}));

type ModuleExports = typeof import("../../src/app/api/settings/devices/[deviceId]/route");

let PATCH: ModuleExports["PATCH"];

const deviceId = "11111111-1111-4111-8111-111111111111";

beforeAll(async () => {
  const routeModule = await import("../../src/app/api/settings/devices/[deviceId]/route");
  PATCH = routeModule.PATCH;
});

beforeEach(() => {
  mockState.reset();
});

function session() {
  return {
    id: "user-1",
    username: "Alice",
    displayName: "Alice",
    avatarUrl: null,
  };
}

function createRequest(headers: Record<string, string> = { Origin: "http://localhost:3000" }) {
  return new Request(`http://localhost:3000/api/settings/devices/${deviceId}`, {
    method: "PATCH",
    headers,
    body: JSON.stringify({ name: "Laptop" }),
  });
}

describe("PATCH /api/settings/devices/[deviceId] CSRF origin checks", () => {
  it("returns 401 for cookie auth when Origin is missing", async () => {
    mockState.getSession.mockResolvedValue(session());
    mockState.setUpdatedRows([{ id: deviceId, deviceKey: "machine", displayName: "Laptop" }]);

    const response = await PATCH(
      createRequest({}),
      { params: Promise.resolve({ deviceId }) }
    );

    expect(response.status).toBe(401);
    expect(await response.json()).toEqual({ error: "Not authenticated" });
    expect(mockState.getSession).not.toHaveBeenCalled();
    expect(mockState.db.update).not.toHaveBeenCalled();
  });

  it("returns 401 for cookie auth when Origin is not allowed", async () => {
    mockState.getSession.mockResolvedValue(session());
    mockState.setUpdatedRows([{ id: deviceId, deviceKey: "machine", displayName: "Laptop" }]);

    const response = await PATCH(
      createRequest({ Origin: "https://attacker.example" }),
      { params: Promise.resolve({ deviceId }) }
    );

    expect(response.status).toBe(401);
    expect(await response.json()).toEqual({ error: "Not authenticated" });
    expect(mockState.getSession).not.toHaveBeenCalled();
    expect(mockState.db.update).not.toHaveBeenCalled();
  });

  it("renames a device when cookie Origin is allowed", async () => {
    mockState.getSession.mockResolvedValue(session());
    mockState.setUpdatedRows([{ id: deviceId, deviceKey: "machine", displayName: "Laptop" }]);

    const response = await PATCH(
      createRequest(),
      { params: Promise.resolve({ deviceId }) }
    );

    expect(response.status).toBe(200);
    expect(await response.json()).toEqual({
      success: true,
      device: { id: deviceId, deviceKey: "machine", displayName: "Laptop" },
    });
    expect(mockState.db.update).toHaveBeenCalledWith(mockState.submittedDevices);
    expect(mockState.revalidateTag).toHaveBeenCalledWith("user:alice", "max");
    expect(mockState.revalidateUsernamePaths).toHaveBeenCalledWith("Alice");
  });
});
