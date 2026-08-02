import { and, eq } from "drizzle-orm";
import { db, groupMembers, type GroupRole } from "@/lib/db";
import { canManageGroupRole, hasGroupRole } from "./utils";

export interface GroupMembership {
  role: GroupRole;
}

export async function getGroupMembership(
  groupId: string,
  userId: string
): Promise<GroupMembership | null> {
  const result = await db
    .select({ role: groupMembers.role })
    .from(groupMembers)
    .where(and(eq(groupMembers.groupId, groupId), eq(groupMembers.userId, userId)))
    .limit(1);

  return result[0] ? { role: result[0].role } : null;
}

export async function requireGroupRole(
  groupId: string,
  userId: string,
  requiredRole: GroupRole
): Promise<GroupMembership | null> {
  const membership = await getGroupMembership(groupId, userId);

  if (!membership || !hasGroupRole(membership.role, requiredRole)) {
    return null;
  }

  return membership;
}

export async function canManageGroupMember(
  groupId: string,
  actorUserId: string,
  targetUserId: string
): Promise<boolean> {
  const [actor, target] = await Promise.all([
    getGroupMembership(groupId, actorUserId),
    getGroupMembership(groupId, targetUserId),
  ]);

  return !!actor && !!target && canManageGroupRole(actor.role, target.role);
}
