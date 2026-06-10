import { NextResponse } from "next/server";
import { and, eq } from "drizzle-orm";
import { db, groupMembers, users } from "@/lib/db";
import { getSessionFromRequest } from "@/lib/auth/requestSession";
import { revalidateGroupCaches } from "@/lib/groups/cache";
import { getGroupMembership } from "@/lib/groups/permissions";
import { getGroupBySlug } from "@/lib/groups/queries";
import { canManageGroupRole } from "@/lib/groups/utils";

const BROWSER_SESSION_OPTIONS = { allowAuthorizationHeader: false } as const;

interface RouteParams {
  params: Promise<{ slug: string }>;
}

async function resolveGroupAccess(request: Request, slug: string) {
  const group = await getGroupBySlug(slug);
  if (!group) {
    return { response: NextResponse.json({ error: "Group not found" }, { status: 404 }) };
  }

  const session = await getSessionFromRequest(request);
  const membership = session ? await getGroupMembership(group.id, session.id) : null;

  if (!group.isPublic && !membership) {
    return { response: NextResponse.json({ error: "Group not found" }, { status: 404 }) };
  }

  return { group, session, membership };
}

export async function GET(request: Request, { params }: RouteParams) {
  try {
    const { slug } = await params;
    const access = await resolveGroupAccess(request, slug);
    if ("response" in access) {
      return access.response;
    }

    const members = await db
      .select({
        id: groupMembers.id,
        userId: users.id,
        username: users.username,
        displayName: users.displayName,
        avatarUrl: users.avatarUrl,
        role: groupMembers.role,
        joinedAt: groupMembers.joinedAt,
      })
      .from(groupMembers)
      .innerJoin(users, eq(groupMembers.userId, users.id))
      .where(eq(groupMembers.groupId, access.group.id));

    return NextResponse.json({ members });
  } catch (error) {
    console.error("List group members error:", error);
    return NextResponse.json({ error: "Failed to fetch group members" }, { status: 500 });
  }
}

export async function DELETE(request: Request, { params }: RouteParams) {
  try {
    const session = await getSessionFromRequest(request, BROWSER_SESSION_OPTIONS);
    if (!session) {
      return NextResponse.json({ error: "Not authenticated" }, { status: 401 });
    }

    const { slug } = await params;
    const group = await getGroupBySlug(slug);
    if (!group) {
      return NextResponse.json({ error: "Group not found" }, { status: 404 });
    }

    const { searchParams } = new URL(request.url);
    const userId = searchParams.get("userId");
    if (!userId) {
      return NextResponse.json({ error: "userId is required" }, { status: 400 });
    }

    if (userId === session.id) {
      return NextResponse.json({ error: "Use the leave endpoint to remove yourself" }, { status: 400 });
    }

    const membership = await getGroupMembership(group.id, session.id);
    if (!group.isPublic && !membership) {
      return NextResponse.json({ error: "Group not found" }, { status: 404 });
    }

    const deleteResult = await db.transaction(async (tx) => {
      const [actor] = await tx
        .select({ role: groupMembers.role })
        .from(groupMembers)
        .where(and(eq(groupMembers.groupId, group.id), eq(groupMembers.userId, session.id)))
        .limit(1)
        .for("update");

      if (!actor) {
        return { status: "forbidden" as const };
      }

      const [target] = await tx
        .select({ role: groupMembers.role })
        .from(groupMembers)
        .where(and(eq(groupMembers.groupId, group.id), eq(groupMembers.userId, userId)))
        .limit(1)
        .for("update");

      if (!target) {
        return { status: "not_found" as const };
      }

      if (!canManageGroupRole(actor.role, target.role)) {
        return { status: "forbidden" as const };
      }

      const deleted = await tx
        .delete(groupMembers)
        .where(and(eq(groupMembers.groupId, group.id), eq(groupMembers.userId, userId)))
        .returning({ id: groupMembers.id });

      return deleted.length === 0
        ? { status: "not_found" as const }
        : { status: "deleted" as const };
    });

    if (deleteResult.status === "forbidden") {
      return NextResponse.json({ error: "Forbidden" }, { status: 403 });
    }

    if (deleteResult.status === "not_found") {
      return NextResponse.json({ error: "Member not found" }, { status: 404 });
    }

    try {
      await revalidateGroupCaches(group.id, group.slug);
    } catch (cacheError) {
      console.error("Remove group member cache invalidation failed:", cacheError);
    }

    return NextResponse.json({ success: true });
  } catch (error) {
    console.error("Remove group member error:", error);
    return NextResponse.json({ error: "Failed to remove member" }, { status: 500 });
  }
}
