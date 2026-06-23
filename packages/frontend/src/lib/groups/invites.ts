import { and, eq, gt } from "drizzle-orm";
import { db, groupInvites, groupMembers, groups, type GroupRole } from "@/lib/db";
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
    public readonly code: "not_found" | "forbidden" | "invalid",
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
}

export interface CreateGroupInviteInput {
  groupId: string;
  invitedBy: string;
  role: GroupRole;
  invitedUsername?: string | null;
  invitedUserId?: string | null;
  expiresAt?: Date;
}

export interface CreatedGroupInvite {
  id: string;
  token: string;
  role: GroupRole;
  invitedUsername: string | null;
  expiresAt: Date;
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
        gt(groupInvites.expiresAt, new Date())
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
  };
}

export async function createGroupInvite({
  groupId,
  invitedBy,
  role,
  invitedUsername = null,
  invitedUserId = null,
  expiresAt = new Date(Date.now() + 14 * 24 * 60 * 60 * 1000),
}: CreateGroupInviteInput): Promise<CreatedGroupInvite> {
  assertInviteRole(role);

  const normalizedUsername = invitedUsername
    ? normalizeInvitedUsername(invitedUsername)
    : null;

  if (normalizedUsername && !isValidGitHubUsername(normalizedUsername)) {
    throw new GroupInviteError("invalid", "Invalid GitHub username");
  }

  const token = createGroupInviteToken();
  const tokenHash = hashGroupInviteToken(token);
  const [created] = await db
    .insert(groupInvites)
    .values({
      groupId,
      invitedBy,
      role,
      invitedUsername: normalizedUsername,
      invitedUsernameNormalized: normalizedUsername,
      invitedUserId,
      tokenHash,
      expiresAt,
    })
    .returning({
      id: groupInvites.id,
      role: groupInvites.role,
      invitedUsername: groupInvites.invitedUsername,
      expiresAt: groupInvites.expiresAt,
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
        status: "accepted",
        acceptedAt,
      })
      .where(
        and(
          eq(groupInvites.id, invite.id),
          eq(groupInvites.status, "pending"),
          gt(groupInvites.expiresAt, acceptedAt)
        )
      )
      .returning({ id: groupInvites.id });

    if (claimed.length === 0) {
      throw new GroupInviteError("not_found", "Invalid or expired invite");
    }

    await tx
      .insert(groupMembers)
      .values({
        groupId: group.id,
        userId: session.id,
        role: invite.role,
        invitedBy: invite.invitedBy,
      })
      .onConflictDoNothing({
        target: [groupMembers.groupId, groupMembers.userId],
      });
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
