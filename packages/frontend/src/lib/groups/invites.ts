import { and, desc, eq, gt, lt, sql } from "drizzle-orm";
import { db, groupInvites, groupInviteRedemptions, groupMembers, groups, type GroupRole } from "@/lib/db";
import type { SessionUser } from "@/lib/auth/session";
import { isValidGitHubUsername } from "@/lib/validation/username";
import {
  canManageGroupRole,
  createGroupInviteToken,
  hashGroupInviteToken,
  isGroupRole,
  normalizeInvitedUsername,
} from "./utils";

export class GroupInviteError extends Error {
  constructor(
    public readonly code: "not_found" | "forbidden" | "invalid" | "conflict",
    message: string
  ) {
    super(message);
  }
}

interface GroupInviteRow {
  invite: typeof groupInvites.$inferSelect;
  group: typeof groups.$inferSelect;
}

export interface GroupInvitePreview {
  group: {
    name: string;
    slug: string;
    isPublic: boolean;
  };
  role: GroupRole;
  invitedUsername: string | null;
  expiresAt: string;
  remainingUses: number;
}

export interface CreateGroupInviteInput {
  groupId: string;
  invitedBy: string;
  role: GroupRole;
  invitedUsername?: string | null;
  invitedUserId?: string | null;
  expiresAt?: Date;
  maxUses?: number;
}

export interface CreatedGroupInvite {
  id: string;
  token: string;
  role: GroupRole;
  invitedUsername: string | null;
  expiresAt: Date;
  maxUses: number;
  useCount: number;
}

function assertInviteRole(role: GroupRole): void {
  if (!isGroupRole(role) || role === "owner") {
    throw new GroupInviteError("invalid", "Invite role must be member or admin");
  }
}

async function findPendingInviteByToken(token: string): Promise<GroupInviteRow | null> {
  const tokenHash = hashGroupInviteToken(token);
  const result = await db
    .select({
      invite: groupInvites,
      group: groups,
    })
    .from(groupInvites)
    .innerJoin(groups, eq(groupInvites.groupId, groups.id))
    .where(
      and(
        eq(groupInvites.tokenHash, tokenHash),
        eq(groupInvites.status, "pending"),
        gt(groupInvites.expiresAt, new Date()),
        lt(groupInvites.useCount, groupInvites.maxUses)
      )
    )
    .limit(1);

  return result[0] ?? null;
}

export async function getGroupInvitePreview(token: string): Promise<GroupInvitePreview> {
  const row = await findPendingInviteByToken(token);

  if (!row) {
    throw new GroupInviteError("not_found", "Invalid or expired invite");
  }

  return {
    group: {
      name: row.group.name,
      slug: row.group.slug,
      isPublic: row.group.isPublic,
    },
    role: row.invite.role,
    invitedUsername: row.invite.invitedUsername,
    expiresAt: row.invite.expiresAt.toISOString(),
    remainingUses: row.invite.maxUses - row.invite.useCount,
  };
}

export async function createGroupInvite({
  groupId,
  invitedBy,
  role,
  invitedUsername = null,
  invitedUserId = null,
  expiresAt = new Date(Date.now() + 14 * 24 * 60 * 60 * 1000),
  maxUses = 1,
}: CreateGroupInviteInput): Promise<CreatedGroupInvite> {
  assertInviteRole(role);

  const normalizedUsername = invitedUsername
    ? normalizeInvitedUsername(invitedUsername) || null
    : null;

  if (normalizedUsername && !isValidGitHubUsername(normalizedUsername)) {
    throw new GroupInviteError("invalid", "Invalid GitHub username");
  }

  if (!Number.isInteger(maxUses) || maxUses < 1 || maxUses > 1000) {
    throw new GroupInviteError("invalid", "Invite uses must be an integer between 1 and 1000");
  }
  if ((normalizedUsername || invitedUserId) && maxUses !== 1) {
    throw new GroupInviteError("invalid", "Account-specific invites must be single-use");
  }
  if (!Number.isFinite(expiresAt.getTime()) || expiresAt.getTime() <= Date.now() || expiresAt.getTime() > Date.now() + 30 * 86400000) {
    throw new GroupInviteError("invalid", "Invite expiry must be within the next 30 days");
  }

  const token = createGroupInviteToken();
  const tokenHash = hashGroupInviteToken(token);
  const created = await db.transaction(async (tx) => {
    const [actor] = await tx.select({ role: groupMembers.role }).from(groupMembers)
      .where(and(eq(groupMembers.groupId, groupId), eq(groupMembers.userId, invitedBy)))
      .limit(1).for("update");
    if (!actor || !canManageGroupRole(actor.role, role)) {
      throw new GroupInviteError("forbidden", "You cannot invite this role");
    }
    const [invite] = await tx.insert(groupInvites).values({
      groupId, invitedBy, role, invitedUsername: normalizedUsername,
      invitedUsernameNormalized: normalizedUsername, invitedUserId, tokenHash,
      expiresAt, maxUses,
    }).returning({
      id: groupInvites.id, role: groupInvites.role, invitedUsername: groupInvites.invitedUsername,
      expiresAt: groupInvites.expiresAt, maxUses: groupInvites.maxUses, useCount: groupInvites.useCount,
    });
    return invite;
  });

  return {
    ...created,
    token,
  };
}

export async function acceptGroupInvite(token: string, session: SessionUser) {
  const row = await findPendingInviteByToken(token);

  if (!row) {
    throw new GroupInviteError("not_found", "Invalid or expired invite");
  }

  const { invite, group } = row;
  const normalizedSessionUsername = normalizeInvitedUsername(session.username);

  if (invite.invitedUserId && invite.invitedUserId !== session.id) {
    throw new GroupInviteError("forbidden", "This invite is not for your account");
  }

  if (
    invite.invitedUsernameNormalized &&
    invite.invitedUsernameNormalized !== normalizedSessionUsername
  ) {
    throw new GroupInviteError("forbidden", "This invite is not for your GitHub username");
  }

  await db.transaction(async (tx) => {
    // Lock the inviter's membership row for the duration of this
    // transaction so a concurrent role change cannot slip between the
    // SELECT and the INSERT below. Without FOR UPDATE, another tx could
    // demote the inviter after this check passes, and we would still
    // insert the new member at the higher role.
    const inviterMembership = await tx
      .select({ role: groupMembers.role })
      .from(groupMembers)
      .where(and(eq(groupMembers.groupId, group.id), eq(groupMembers.userId, invite.invitedBy)))
      .limit(1)
      .for("update");
    if (!inviterMembership[0] || !canManageGroupRole(inviterMembership[0].role, invite.role)) {
      throw new GroupInviteError("forbidden", "Inviter no longer has permission to grant this role");
    }

    // Capture `acceptedAt` AFTER the FOR UPDATE lock has been acquired so
    // an invite that expires while the request was queued is rejected.
    // A pre-transaction timestamp could pass `expiresAt > acceptedAt`
    // even though wall-clock time has now advanced past `expiresAt`.
    const acceptedAt = new Date();

    const claimed = await tx
      .update(groupInvites)
      .set({
        useCount: sql`${groupInvites.useCount} + 1`,
        status: sql`CASE WHEN ${groupInvites.useCount} + 1 = ${groupInvites.maxUses} THEN 'accepted' ELSE 'pending' END`,
        acceptedAt: sql`CASE WHEN ${groupInvites.useCount} + 1 = ${groupInvites.maxUses} THEN clock_timestamp() ELSE NULL END`,
      })
      .where(
        and(
          eq(groupInvites.id, invite.id),
          eq(groupInvites.status, "pending"),
          gt(groupInvites.expiresAt, acceptedAt),
          gt(groupInvites.expiresAt, sql`clock_timestamp()`),
          lt(groupInvites.useCount, groupInvites.maxUses)
        )
      )
      .returning({ id: groupInvites.id });

    if (claimed.length === 0) {
      throw new GroupInviteError("not_found", "Invalid or expired invite");
    }

    const [redemption] = await tx.insert(groupInviteRedemptions)
      .values({ inviteId: invite.id, userId: session.id })
      .onConflictDoNothing({ target: [groupInviteRedemptions.inviteId, groupInviteRedemptions.userId] })
      .returning({ inviteId: groupInviteRedemptions.inviteId });
    if (!redemption) {
      throw new GroupInviteError("conflict", "This account has already used this invite");
    }

    const [membership] = await tx
      .insert(groupMembers)
      .values({
        groupId: group.id,
        userId: session.id,
        role: invite.role,
        invitedBy: invite.invitedBy,
      })
      .onConflictDoNothing({
        target: [groupMembers.groupId, groupMembers.userId],
      }).returning({ userId: groupMembers.userId });
    if (!membership) {
      throw new GroupInviteError("conflict", "You are already a member of this group");
    }
  });

  return {
    group: {
      id: group.id,
      name: group.name,
      slug: group.slug,
    },
    role: invite.role,
  };
}


/** Management responses never return token hashes or recoverable join links. */
export async function listGroupInvites(groupId: string, actorRole: GroupRole) {
  if (actorRole === "member") throw new GroupInviteError("forbidden", "Forbidden");
  return db.select({
    id: groupInvites.id, role: groupInvites.role, invitedUsername: groupInvites.invitedUsername,
    status: groupInvites.status, maxUses: groupInvites.maxUses, useCount: groupInvites.useCount,
    expiresAt: groupInvites.expiresAt, createdAt: groupInvites.createdAt,
  }).from(groupInvites).where(and(eq(groupInvites.groupId, groupId),
    actorRole === "admin" ? eq(groupInvites.role, "member") : undefined))
    .orderBy(desc(groupInvites.createdAt), desc(groupInvites.id)).limit(100);
}

export async function revokeGroupInvite(groupId: string, inviteId: string, actorId: string) {
  await db.transaction(async (tx) => {
    // Same membership-before-invite lock order as redemption. A concurrent
    // redemption either commits before revocation or observes revoked status.
    const [actor] = await tx.select({ role: groupMembers.role }).from(groupMembers)
      .where(and(eq(groupMembers.groupId, groupId), eq(groupMembers.userId, actorId)))
      .limit(1).for("update");
    if (!actor || actor.role === "member") throw new GroupInviteError("forbidden", "Forbidden");
    const [revoked] = await tx.update(groupInvites).set({ status: "revoked" })
      .where(and(eq(groupInvites.groupId, groupId), eq(groupInvites.id, inviteId),
        eq(groupInvites.status, "pending"), actor.role === "admin" ? eq(groupInvites.role, "member") : undefined))
      .returning({ id: groupInvites.id });
    if (!revoked) throw new GroupInviteError("not_found", "Active invite not found");
  });
}
