import type { GroupRole } from "@/lib/db/schema";

const ROLE_LEVEL: Record<GroupRole, number> = { owner: 3, admin: 2, member: 1 };

export function hasGroupRole(userRole: GroupRole, requiredRole: GroupRole): boolean {
  return ROLE_LEVEL[userRole] >= ROLE_LEVEL[requiredRole];
}

export function canManageGroupRole(actorRole: GroupRole, targetRole: GroupRole): boolean {
  return ROLE_LEVEL[actorRole] > ROLE_LEVEL[targetRole];
}
