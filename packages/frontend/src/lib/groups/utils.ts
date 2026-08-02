import { generateRandomString, hashToken } from "../auth/utils";
import { groupRoles, type GroupRole } from "../db/schema";

export const GROUP_SLUG_MAX_LENGTH = 100;

const RESERVED_GROUP_SLUGS = new Set(["new", "join", "settings", "members"]);
const ROLE_LEVEL: Record<GroupRole, number> = {
  owner: 3,
  admin: 2,
  member: 1,
};

interface SlugOptions {
  suffix?: string | number;
}

export function isGroupRole(role: unknown): role is GroupRole {
  return typeof role === "string" && groupRoles.includes(role as GroupRole);
}

export function hasGroupRole(userRole: GroupRole, requiredRole: GroupRole): boolean {
  return ROLE_LEVEL[userRole] >= ROLE_LEVEL[requiredRole];
}

export function canManageGroupRole(actorRole: GroupRole, targetRole: GroupRole): boolean {
  return ROLE_LEVEL[actorRole] > ROLE_LEVEL[targetRole];
}

export function normalizeInvitedUsername(username: string): string {
  return username.trim().replace(/^@/, "").toLowerCase();
}

export function slugifyGroupName(name: string, options: SlugOptions = {}): string {
  const rawBase = name
    .toLowerCase()
    .trim()
    .replace(/[\s_]+/g, "-")
    .replace(/[^a-z0-9-]/g, "")
    .replace(/-+/g, "-")
    .replace(/^-|-$/g, "");

  let base = rawBase;
  if (!base || RESERVED_GROUP_SLUGS.has(base)) {
    const suffix = generateRandomString(8);
    base = base ? `${base}-${suffix}` : `group-${suffix}`;
  }

  const suffix = options.suffix === undefined ? "" : `-${options.suffix}`;
  const maxBaseLength = GROUP_SLUG_MAX_LENGTH - suffix.length;
  const trimmedBase = base
    .slice(0, Math.max(1, maxBaseLength))
    .replace(/-+$/g, "");

  return `${trimmedBase}${suffix}`.slice(0, GROUP_SLUG_MAX_LENGTH);
}

export function createGroupInviteToken(): string {
  return `tg_${generateRandomString(48)}`;
}

export function hashGroupInviteToken(token: string): string {
  return hashToken(token);
}
