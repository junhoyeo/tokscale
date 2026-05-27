import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

const mockState = vi.hoisted(() => {
  const pendingRows: Array<Record<string, unknown>> = [];
  let claimedRows: Array<Record<string, unknown>> = [];

  // inviterRows: default to owner role so existing tests pass without change
  const inviterRows: Array<Record<string, unknown>> = [{ role: "owner" }];

  const tables = {
    groupInvites: {
      id: "groupInvites.id",
      groupId: "groupInvites.groupId",
      invitedUsername: "groupInvites.invitedUsername",
      invitedUsernameNormalized: "groupInvites.invitedUsernameNormalized",
      invitedUserId: "groupInvites.invitedUserId",
      invitedBy: "groupInvites.invitedBy",
      role: "groupInvites.role",
      status: "groupInvites.status",
      tokenHash: "groupInvites.tokenHash",
      expiresAt: "groupInvites.expiresAt",
      acceptedAt: "groupInvites.acceptedAt",
    },
    groupMembers: {
      groupId: "groupMembers.groupId",
      userId: "groupMembers.userId",
      role: "groupMembers.role",
    },
    groups: {
      id: "groups.id",
    },
  };

  const and = vi.fn((...conditions: unknown[]) => ({ kind: "and", conditions }));
  const eq = vi.fn((left: unknown, right: unknown) => ({ kind: "eq", left, right }));
  const gt = vi.fn((left: unknown, right: unknown) => ({ kind: "gt", left, right }));

  const limit = vi.fn(async () => pendingRows);
  const where = vi.fn(() => ({ limit }));
  const innerJoin = vi.fn(() => ({ where }));
  const from = vi.fn(() => ({ innerJoin }));

  // tx.select used by the inviter re-check (B1).
  // Chain: tx.select().from().where().limit().for("update")
  const txForUpdate = vi.fn(async () => inviterRows);
  const txLimit = vi.fn(() => ({ for: txForUpdate }));
  const txWhere = vi.fn(() => ({ limit: txLimit }));
  const txFrom = vi.fn(() => ({ where: txWhere }));

  const claimReturning = vi.fn(async () => claimedRows);
  const updateWhere = vi.fn(() => ({ returning: claimReturning }));
  const updateSet = vi.fn(() => ({ where: updateWhere }));

  const onConflictDoNothing = vi.fn(async () => undefined);
  const insertValues = vi.fn(() => ({ onConflictDoNothing }));

  const tx = {
    select: vi.fn(() => ({ from: txFrom })),
    update: vi.fn(() => ({ set: updateSet })),
    insert: vi.fn(() => ({ values: insertValues })),
  };

  const db = {
    select: vi.fn(() => ({ from })),
    transaction: vi.fn(async (callback: (txArg: typeof tx) => Promise<unknown>) =>
      callback(tx)
    ),
  };

  return {
    tables,
    and,
    eq,
    gt,
    db,
    tx,
    updateSet,
    updateWhere,
    claimReturning,
    insertValues,
    onConflictDoNothing,
    inviterRows,
    reset() {
      pendingRows.length = 0;
      claimedRows = [];
      // restore default inviter row (owner) so existing tests keep passing
      inviterRows.length = 0;
      inviterRows.push({ role: "owner" });
      and.mockClear();
      eq.mockClear();
      gt.mockClear();
      limit.mockClear();
      where.mockClear();
      innerJoin.mockClear();
      from.mockClear();
      txForUpdate.mockClear();
      txLimit.mockClear();
      txWhere.mockClear();
      txFrom.mockClear();
      db.select.mockClear();
      db.transaction.mockClear();
      tx.select.mockClear();
      tx.update.mockClear();
      tx.insert.mockClear();
      updateSet.mockClear();
      updateWhere.mockClear();
      claimReturning.mockClear();
      insertValues.mockClear();
      onConflictDoNothing.mockClear();
    },
    setPendingRow(row: Record<string, unknown> | null) {
      pendingRows.length = 0;
      if (row) {
        pendingRows.push(row);
      }
    },
    setClaimedRows(rows: Array<Record<string, unknown>>) {
      claimedRows = rows;
    },
  };
});

vi.mock("drizzle-orm", () => ({
  and: mockState.and,
  eq: mockState.eq,
  gt: mockState.gt,
}));

vi.mock("@/lib/db", () => ({
  db: mockState.db,
  groupInvites: mockState.tables.groupInvites,
  groupMembers: mockState.tables.groupMembers,
  groups: mockState.tables.groups,
}));

vi.mock("../../src/lib/db/schema", () => ({
  groupRoles: ["owner", "admin", "member"],
}));

vi.mock("@/lib/validation/username", () => ({
  isValidGitHubUsername: vi.fn(() => true),
}));

type ModuleExports = typeof import("../../src/lib/groups/invites");

let acceptGroupInvite: ModuleExports["acceptGroupInvite"];

beforeAll(async () => {
  const inviteModule = await import("../../src/lib/groups/invites");
  acceptGroupInvite = inviteModule.acceptGroupInvite;
});

beforeEach(() => {
  mockState.reset();
});

function pendingInvite() {
  return {
    invite: {
      id: "invite-1",
      groupId: "group-1",
      invitedUsername: null,
      invitedUsernameNormalized: null,
      invitedUserId: null,
      invitedBy: "owner-1",
      role: "member",
      status: "pending",
      tokenHash: "hash",
      expiresAt: new Date("2026-06-01T00:00:00Z"),
      acceptedAt: null,
      createdAt: new Date("2026-05-01T00:00:00Z"),
    },
    group: {
      id: "group-1",
      name: "Team",
      slug: "team",
      isPublic: false,
    },
  };
}

describe("group invites", () => {
  it("claims a pending invite inside the transaction before adding membership", async () => {
    mockState.setPendingRow(pendingInvite());
    mockState.setClaimedRows([{ id: "invite-1" }]);

    const accepted = await acceptGroupInvite("tg_token", {
      id: "user-1",
      username: "alice",
      displayName: null,
      avatarUrl: null,
    });

    expect(accepted).toEqual({
      group: { id: "group-1", name: "Team", slug: "team" },
      role: "member",
    });
    expect(mockState.tx.update).toHaveBeenCalledWith(mockState.tables.groupInvites);
    expect(mockState.eq).toHaveBeenCalledWith(mockState.tables.groupInvites.status, "pending");
    expect(mockState.gt).toHaveBeenCalledWith(
      mockState.tables.groupInvites.expiresAt,
      expect.any(Date)
    );
    expect(mockState.tx.insert).toHaveBeenCalledWith(mockState.tables.groupMembers);
    expect(mockState.onConflictDoNothing).toHaveBeenCalledWith({
      target: [mockState.tables.groupMembers.groupId, mockState.tables.groupMembers.userId],
    });
  });

  it("rejects the accept when another request already claimed the invite", async () => {
    mockState.setPendingRow(pendingInvite());
    mockState.setClaimedRows([]);

    await expect(
      acceptGroupInvite("tg_token", {
        id: "user-2",
        username: "bob",
        displayName: null,
        avatarUrl: null,
      })
    ).rejects.toMatchObject({
      code: "not_found",
      message: "Invalid or expired invite",
    });

    expect(mockState.tx.insert).not.toHaveBeenCalled();
  });
});
