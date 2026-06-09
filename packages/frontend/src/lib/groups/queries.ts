import { and, desc, eq, sql } from "drizzle-orm";
import { db, groupMembers, groups, type Group, type GroupRole } from "@/lib/db";

export interface GroupSummary extends Group {
  memberCount: number;
  role?: GroupRole;
}

function mapGroupSummary(row: Group & { memberCount?: number | string; role?: GroupRole }): GroupSummary {
  return {
    ...row,
    memberCount: Number(row.memberCount) || 0,
    role: row.role,
  };
}

export async function getGroupBySlug(slug: string): Promise<Group | null> {
  const result = await db.select().from(groups).where(eq(groups.slug, slug)).limit(1);
  return result[0] ?? null;
}

export async function getGroupMemberCount(groupId: string): Promise<number> {
  const result = await db
    .select({ count: sql<number>`CAST(COUNT(*) AS integer)`.as("count") })
    .from(groupMembers)
    .where(eq(groupMembers.groupId, groupId));

  return Number(result[0]?.count) || 0;
}

export async function listPublicGroups(page: number, limit: number) {
  const offset = (page - 1) * limit;
  const [items, countRows] = await Promise.all([
    db
      .select({
        id: groups.id,
        name: groups.name,
        slug: groups.slug,
        description: groups.description,
        avatarUrl: groups.avatarUrl,
        isPublic: groups.isPublic,
        createdBy: groups.createdBy,
        createdAt: groups.createdAt,
        updatedAt: groups.updatedAt,
        memberCount: sql<number>`CAST(COUNT(${groupMembers.id}) AS integer)`.as("member_count"),
      })
      .from(groups)
      .leftJoin(groupMembers, eq(groupMembers.groupId, groups.id))
      .where(eq(groups.isPublic, true))
      .groupBy(groups.id)
      .orderBy(desc(groups.updatedAt), desc(groups.id))
      .limit(limit)
      .offset(offset),
    db
      .select({ count: sql<number>`CAST(COUNT(*) AS integer)`.as("count") })
      .from(groups)
      .where(eq(groups.isPublic, true)),
  ]);

  const total = Number(countRows[0]?.count) || 0;

  return {
    groups: items.map(mapGroupSummary),
    pagination: {
      page,
      limit,
      total,
      totalPages: Math.ceil(total / limit),
      hasNext: offset + items.length < total,
      hasPrev: page > 1,
    },
  };
}

export async function listUserGroups(userId: string, page: number, limit: number) {
  const offset = (page - 1) * limit;
  const [items, countRows] = await Promise.all([
    db
      .select({
        id: groups.id,
        name: groups.name,
        slug: groups.slug,
        description: groups.description,
        avatarUrl: groups.avatarUrl,
        isPublic: groups.isPublic,
        createdBy: groups.createdBy,
        createdAt: groups.createdAt,
        updatedAt: groups.updatedAt,
        role: groupMembers.role,
        memberCount: sql<number>`CAST((SELECT COUNT(*) FROM "group_members" WHERE "group_members"."group_id" = ${groups.id}) AS integer)`.as("member_count"),
      })
      .from(groupMembers)
      .innerJoin(groups, eq(groupMembers.groupId, groups.id))
      .where(eq(groupMembers.userId, userId))
      .orderBy(desc(groups.updatedAt), desc(groups.id))
      .limit(limit)
      .offset(offset),
    db
      .select({ count: sql<number>`CAST(COUNT(*) AS integer)`.as("count") })
      .from(groupMembers)
      .where(eq(groupMembers.userId, userId)),
  ]);

  const total = Number(countRows[0]?.count) || 0;

  return {
    groups: items.map(mapGroupSummary),
    pagination: {
      page,
      limit,
      total,
      totalPages: Math.ceil(total / limit),
      hasNext: offset + items.length < total,
      hasPrev: page > 1,
    },
  };
}

export async function isGroupMember(groupId: string, userId: string): Promise<boolean> {
  const result = await db
    .select({ id: groupMembers.id })
    .from(groupMembers)
    .where(and(eq(groupMembers.groupId, groupId), eq(groupMembers.userId, userId)))
    .limit(1);

  return result.length > 0;
}
