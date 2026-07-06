import { beforeEach, describe, expect, it, vi } from "vitest";

// Mock state must be hoisted so vi.mock factories can reference it.
const mockState = vi.hoisted(() => {
  const orderByCalls: unknown[][] = [];

  const tables = {
    groups: {
      id: "groups.id",
      name: "groups.name",
      slug: "groups.slug",
      description: "groups.description",
      avatarUrl: "groups.avatarUrl",
      isPublic: "groups.isPublic",
      createdBy: "groups.createdBy",
      createdAt: "groups.createdAt",
      updatedAt: "groups.updatedAt",
    },
    groupMembers: {
      id: "groupMembers.id",
      groupId: "groupMembers.groupId",
      userId: "groupMembers.userId",
      role: "groupMembers.role",
    },
  };

  const eq = vi.fn(() => "eq");
  const desc = vi.fn((col: unknown) => `desc(${String(col)})`);
  const sql = Object.assign(
    vi.fn((_strings: TemplateStringsArray) => ({ as: () => ({}) })),
    { raw: vi.fn() }
  );

  // Each call to db.select() returns a builder that accumulates orderBy args
  // and resolves with an empty array (enough to exercise the sort path).
  const db = {
    select: vi.fn(() => {
      const builder = {
        from: vi.fn(() => builder),
        innerJoin: vi.fn(() => builder),
        leftJoin: vi.fn(() => builder),
        where: vi.fn(() => builder),
        groupBy: vi.fn(() => builder),
        orderBy: vi.fn((...args: unknown[]) => {
          orderByCalls.push(args);
          return builder;
        }),
        limit: vi.fn(() => builder),
        offset: vi.fn(() => builder),
        then: (resolve: (v: unknown[]) => unknown) => resolve([]),
      };
      return builder;
    }),
  };

  return {
    db,
    tables,
    orderByCalls,
    eq,
    desc,
    sql,
    reset() {
      orderByCalls.length = 0;
      db.select.mockClear();
      eq.mockClear();
      desc.mockClear();
      sql.mockClear();
    },
  };
});

vi.mock("@/lib/db", () => ({
  db: mockState.db,
  groups: mockState.tables.groups,
  groupMembers: mockState.tables.groupMembers,
}));

vi.mock("drizzle-orm", () => ({
  eq: mockState.eq,
  desc: mockState.desc,
  and: vi.fn(() => "and"),
  sql: mockState.sql,
}));

type QueriesModule = typeof import("../../src/lib/groups/queries");
let listPublicGroups: QueriesModule["listPublicGroups"];
let listUserGroups: QueriesModule["listUserGroups"];

beforeEach(async () => {
  mockState.reset();
  // Re-import fresh each time so hoisted mocks are applied.
  vi.resetModules();
  const mod = await import("../../src/lib/groups/queries");
  listPublicGroups = mod.listPublicGroups;
  listUserGroups = mod.listUserGroups;
});

describe("listPublicGroups pagination stable sort", () => {
  it("orders by updatedAt DESC then id DESC to prevent row duplication on ties", async () => {
    await listPublicGroups(1, 20);

    // The main items query fires orderBy; the count query does not.
    const itemsOrderBy = mockState.orderByCalls[0];
    expect(itemsOrderBy).toHaveLength(2);
    // First arg: desc(groups.updatedAt)
    expect(mockState.desc).toHaveBeenCalledWith(mockState.tables.groups.updatedAt);
    // Second arg: desc(groups.id)
    expect(mockState.desc).toHaveBeenCalledWith(mockState.tables.groups.id);
  });

  it("maintains stable ordering across paginated fetches when updatedAt ties", async () => {
    // Simulate groups with identical updatedAt — stable sort must use id as
    // tiebreaker so page 1 and page 2 never overlap or skip rows.
    await listPublicGroups(1, 2);
    mockState.reset();
    await listPublicGroups(2, 2);

    // Both fetches must use the same two-column sort.
    for (const call of mockState.orderByCalls) {
      expect(call).toHaveLength(2);
    }
    expect(mockState.desc).toHaveBeenCalledWith(mockState.tables.groups.updatedAt);
    expect(mockState.desc).toHaveBeenCalledWith(mockState.tables.groups.id);
  });
});

describe("listUserGroups pagination stable sort", () => {
  it("orders by updatedAt DESC then id DESC to prevent row duplication on ties", async () => {
    await listUserGroups("user-1", 1, 20);

    const itemsOrderBy = mockState.orderByCalls[0];
    expect(itemsOrderBy).toHaveLength(2);
    expect(mockState.desc).toHaveBeenCalledWith(mockState.tables.groups.updatedAt);
    expect(mockState.desc).toHaveBeenCalledWith(mockState.tables.groups.id);
  });
});
