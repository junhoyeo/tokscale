import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

const mockState = vi.hoisted(() => {
  const updateResults: Array<Array<Record<string, unknown>>> = [];
  const updateSets: Array<Record<string, unknown>> = [];
  const updateWhereConditions: unknown[] = [];
  const getSession = vi.fn();

  const tables = {
    deviceCodes: {
      id: "deviceCodes.id",
      userCode: "deviceCodes.userCode",
      expiresAt: "deviceCodes.expiresAt",
      userId: "deviceCodes.userId",
    },
  };

  const eq = vi.fn((left: unknown, right: unknown) => ({ kind: "eq", left, right }));
  const and = vi.fn((...terms: unknown[]) => ({ kind: "and", terms }));
  const gt = vi.fn((left: unknown, right: unknown) => ({ kind: "gt", left, right }));
  const isNull = vi.fn((left: unknown) => ({ kind: "isNull", left }));

  function nextUpdateResult() {
    return updateResults.shift() ?? [];
  }

  const db = {
    select: vi.fn(() => {
      const builder = {
        from: vi.fn(() => builder),
        where: vi.fn(() => builder),
        limit: vi.fn(async () => []),
      };

      return builder;
    }),
    update: vi.fn(() => {
      const builder = {
        set: vi.fn((values: Record<string, unknown>) => {
          updateSets.push(values);
          return builder;
        }),
        where: vi.fn((condition: unknown) => {
          updateWhereConditions.push(condition);
          return builder;
        }),
        returning: vi.fn(async () => nextUpdateResult()),
      };

      return builder;
    }),
  };

  return {
    db,
    tables,
    eq,
    and,
    gt,
    isNull,
    getSession,
    updateSets,
    updateWhereConditions,
    reset() {
      updateResults.length = 0;
      updateSets.length = 0;
      updateWhereConditions.length = 0;
      db.select.mockClear();
      db.update.mockClear();
      getSession.mockReset();
      eq.mockClear();
      and.mockClear();
      gt.mockClear();
      isNull.mockClear();
    },
    pushUpdateResult(rows: Array<Record<string, unknown>>) {
      updateResults.push(rows);
    },
  };
});

vi.mock("@/lib/db", () => ({
  db: mockState.db,
  deviceCodes: mockState.tables.deviceCodes,
}));

vi.mock("drizzle-orm", () => ({
  eq: mockState.eq,
  and: mockState.and,
  gt: mockState.gt,
  isNull: mockState.isNull,
}));

vi.mock("@/lib/auth/session", () => ({
  getSession: mockState.getSession,
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

function flattenedTerms(condition: unknown): Array<Record<string, unknown>> {
  if (!condition || typeof condition !== "object") {
    return [];
  }

  const record = condition as Record<string, unknown>;
  if (record.kind === "and" && Array.isArray(record.terms)) {
    return record.terms.flatMap((term) => flattenedTerms(term));
  }

  return [record];
}

function createAuthorizeRequest(userCode: string) {
  return new Request("http://localhost:3000/api/auth/device/authorize", {
    method: "POST",
    headers: { Origin: "http://localhost:3000" },
    body: JSON.stringify({ userCode }),
  });
}

describe("POST /api/auth/device/authorize", () => {
  it("claims a valid device code with a guarded update", async () => {
    mockState.getSession.mockResolvedValue({
      id: "user-1",
      username: "alice",
      displayName: "Alice",
      avatarUrl: null,
    });
    mockState.pushUpdateResult([{ id: "device-1" }]);

    const response = await POST(createAuthorizeRequest("ABCD-1234"));
    const body = await response.json();

    expect(response.status).toBe(200);
    expect(body).toEqual({ success: true });
    expect(mockState.db.select).not.toHaveBeenCalled();
    expect(mockState.db.update).toHaveBeenCalledWith(mockState.tables.deviceCodes);
    expect(mockState.updateSets).toEqual([{ userId: "user-1" }]);
  });

  it("returns invalid or expired when the atomic device-code claim loses the race", async () => {
    mockState.getSession.mockResolvedValue({
      id: "user-1",
      username: "alice",
      displayName: "Alice",
      avatarUrl: null,
    });
    mockState.pushUpdateResult([]);

    const response = await POST(createAuthorizeRequest("abcd1234"));
    const body = await response.json();
    const updateTerms = flattenedTerms(mockState.updateWhereConditions[0]);

    expect(mockState.db.update).toHaveBeenCalledWith(mockState.tables.deviceCodes);
    expect(mockState.updateSets).toEqual([{ userId: "user-1" }]);
    expect(updateTerms).toEqual(
      expect.arrayContaining([
        { kind: "eq", left: mockState.tables.deviceCodes.userCode, right: "ABCD-1234" },
        expect.objectContaining({ kind: "gt", left: mockState.tables.deviceCodes.expiresAt }),
        { kind: "isNull", left: mockState.tables.deviceCodes.userId },
      ])
    );
    expect(response.status).toBe(400);
    expect(body).toEqual({ error: "Invalid or expired code" });
    expect(mockState.db.select).not.toHaveBeenCalled();
  });
});
