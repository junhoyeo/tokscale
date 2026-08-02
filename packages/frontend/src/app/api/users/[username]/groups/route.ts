import { NextResponse } from "next/server";
import { and, eq, sql } from "drizzle-orm";
import { db, groupMembers, groups, users } from "@/lib/db";
import { getSessionFromRequest } from "@/lib/auth/requestSession";
import {
  USERNAME_LOOKUP_LIMIT,
  getSingleUsernameMatch,
  usernameEqualsIgnoreCase,
} from "@/lib/db/usernameLookup";

export async function GET(
  request: Request,
  { params }: { params: Promise<{ username: string }> }
) {
  try {
    const { username } = await params;
    const session = await getSessionFromRequest(request);
    const userRows = await db
      .select({ id: users.id, username: users.username })
      .from(users)
      .where(usernameEqualsIgnoreCase(username))
      .limit(USERNAME_LOOKUP_LIMIT);
    const user = getSingleUsernameMatch(userRows, username);

    if (!user) {
      return NextResponse.json({ error: "User not found" }, { status: 404 });
    }

    const showPrivate = session?.id === user.id;
    const rows = await db
      .select({
        id: groups.id,
        name: groups.name,
        slug: groups.slug,
        description: groups.description,
        avatarUrl: groups.avatarUrl,
        isPublic: groups.isPublic,
        role: groupMembers.role,
        memberCount: sql<number>`CAST((SELECT COUNT(*) FROM "group_members" WHERE "group_members"."group_id" = ${groups.id}) AS integer)`.as("member_count"),
      })
      .from(groupMembers)
      .innerJoin(groups, eq(groupMembers.groupId, groups.id))
      .where(
        showPrivate
          ? eq(groupMembers.userId, user.id)
          : and(eq(groupMembers.userId, user.id), eq(groups.isPublic, true))
      );

    return NextResponse.json({
      groups: rows.map((row) => ({
        ...row,
        memberCount: Number(row.memberCount) || 0,
      })),
    });
  } catch (error) {
    console.error("Fetch user groups error:", error);
    return NextResponse.json({ error: "Failed to fetch user groups" }, { status: 500 });
  }
}
