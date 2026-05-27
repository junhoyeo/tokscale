import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

/**
 * B1: Inviter role re-check inside acceptGroupInvite transaction.
 * Exploit: Alice promotes Bob→admin, Bob mints an admin invite for Carol,
 * Alice demotes Bob→member, Carol accepts → should be rejected.
 */

const mockState = vi.hoisted(() => {
  const pendingRows: Array<Record<string, unknown>> = [];
  let claimedRows: Array<Record<string, unknown>> = [];
  const inviterRows: Array<Record<string, unknown>> = [];

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

  // Outer db.select for findPendingInviteByToken
  const outerLimit = vi.fn(async () => pendingRows);
  const outerWhere = vi.fn(() => ({ limit: outerLimit }));
  const outerInnerJoin = vi.fn(() => ({ where: outerWhere }));
  const outerFrom = vi.fn(() => ({ innerJoin: outerInnerJoin }));

  // tx.select for inviterMembership check.
  // Chain: tx.select().from().where().limit().for("update")
  let inviterCallCount = 0;
  const txForUpdate = vi.fn(async () => {
    inviterCallCount++;
    return inviterRows;
  });
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
    select: vi.fn(() => ({ from: outerFrom })),
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
      inviterRows.length = 0;
      inviterCallCount = 0;
      and.mockClear();
      eq.mockClear();
      gt.mockClear();
      outerLimit.mockClear();
      outerWhere.mockClear();
      outerInnerJoin.mockClear();
      outerFrom.mockClear();
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
    setInviterRow(row: Record<string, unknown> | null) {
      inviterRows.length = 0;
      if (row) {
        inviterRows.push(row);
      }
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

function adminInvite() {
  return {
    invite: {
      id: "invite-admin",
      groupId: "group-1",
      invitedUsername: null,
      invitedUsernameNormalized: null,
      invitedUserId: null,
      invitedBy: "bob",
      role: "admin",
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

describe("acceptGroupInvite — inviter role re-check", () => {
  it("rejects accept when inviter has been demoted below the invite role", async () => {
    // Bob minted an admin invite but has since been demoted to member
    mockState.setPendingRow(adminInvite());
    mockState.setInviterRow({ role: "member" }); // Bob is now a member
    mockState.setClaimedRows([{ id: "invite-admin" }]);

    await expect(
      acceptGroupInvite("tg_token", {
        id: "carol",
        username: "carol",
        displayName: null,
        avatarUrl: null,
      })
    ).rejects.toMatchObject({
      code: "forbidden",
      message: "Inviter no longer has permission to grant this role",
    });

    // Transaction should not proceed to insert membership
    expect(mockState.tx.insert).not.toHaveBeenCalled();
  });

  it("rejects accept when inviter is no longer a member of the group", async () => {
    // Bob minted an admin invite but has since left the group
    mockState.setPendingRow(adminInvite());
    mockState.setInviterRow(null); // Bob is not in the group
    mockState.setClaimedRows([{ id: "invite-admin" }]);

    await expect(
      acceptGroupInvite("tg_token", {
        id: "carol",
        username: "carol",
        displayName: null,
        avatarUrl: null,
      })
    ).rejects.toMatchObject({
      code: "forbidden",
      message: "Inviter no longer has permission to grant this role",
    });

    expect(mockState.tx.insert).not.toHaveBeenCalled();
  });

  it("allows accept when inviter still has sufficient role (admin inviting admin)", async () => {
    mockState.setPendingRow(adminInvite());
    mockState.setInviterRow({ role: "admin" }); // Bob is still admin
    mockState.setClaimedRows([{ id: "invite-admin" }]);

    // canManageGroupRole("admin", "admin") = false (admin cannot manage same-level)
    // So this should fail too — admin cannot grant admin role
    await expect(
      acceptGroupInvite("tg_token", {
        id: "carol",
        username: "carol",
        displayName: null,
        avatarUrl: null,
      })
    ).rejects.toMatchObject({
      code: "forbidden",
      message: "Inviter no longer has permission to grant this role",
    });
  });

  it("allows accept when owner invites a member-role invite", async () => {
    const memberInvite = {
      invite: {
        id: "invite-member",
        groupId: "group-1",
        invitedUsername: null,
        invitedUsernameNormalized: null,
        invitedUserId: null,
        invitedBy: "owner-1",
        role: "member",
        status: "pending",
        tokenHash: "hash2",
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
    mockState.setPendingRow(memberInvite);
    mockState.setInviterRow({ role: "owner" }); // owner still in group
    mockState.setClaimedRows([{ id: "invite-member" }]);

    const result = await acceptGroupInvite("tg_token", {
      id: "carol",
      username: "carol",
      displayName: null,
      avatarUrl: null,
    });

    expect(result).toEqual({
      group: { id: "group-1", name: "Team", slug: "team" },
      role: "member",
    });
    expect(mockState.tx.insert).toHaveBeenCalledWith(mockState.tables.groupMembers);
  });
});
