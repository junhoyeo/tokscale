import { NextResponse } from "next/server";
import { and, eq, ne, sql } from "drizzle-orm";
import { db, groupMembers } from "@/lib/db";
import { getSessionFromRequest } from "@/lib/auth/requestSession";
import { revalidateGroupCaches } from "@/lib/groups/cache";
import { getGroupMembership } from "@/lib/groups/permissions";
import { getGroupBySlug } from "@/lib/groups/queries";
import { canManageGroupRole, isGroupRole } from "@/lib/groups/utils";
import type { GroupRole } from "@/lib/db";

interface RouteParams {
  params: Promise<{ slug: string; userId: string }>;
}

export async function PATCH(request: Request, { params }: RouteParams) {
  try {
    const session = await getSessionFromRequest(request);
    if (!session) {
      return NextResponse.json({ error: "Not authenticated" }, { status: 401 });
    }

    const { slug, userId } = await params;
    const group = await getGroupBySlug(slug);
    if (!group) {
      return NextResponse.json({ error: "Group not found" }, { status: 404 });
    }

    const body = await request.json().catch(() => null);
    if (!body || typeof body !== "object" || Array.isArray(body)) {
      return NextResponse.json({ error: "Invalid JSON body" }, { status: 400 });
    }

    const nextRole = (body as Record<string, unknown>).role;
    if (!isGroupRole(nextRole) || nextRole === "owner") {
      return NextResponse.json({ error: "Role must be member or admin" }, { status: 400 });
    }

    const [actor, target] = await Promise.all([
      getGroupMembership(group.id, session.id),
      getGroupMembership(group.id, userId),
    ]);

    if (!group.isPublic && !actor) {
      return NextResponse.json({ error: "Group not found" }, { status: 404 });
    }

    if (
      !actor ||
      !target ||
      !canManageGroupRole(actor.role, target.role) ||
      !canManageGroupRole(actor.role, nextRole)
    ) {
      return NextResponse.json({ error: "Forbidden" }, { status: 403 });
    }

    // Demoting an owner would orphan the group if no other owners remain.
    // The route already rejects nextRole === "owner" above, so reaching this
    // branch with target.role === "owner" always means we're demoting.
    if (target.role === "owner") {
      const [{ count: otherOwnerCount }] = await db
        .select({ count: sql<number>`COUNT(*)::int` })
        .from(groupMembers)
        .where(
          and(
            eq(groupMembers.groupId, group.id),
            eq(groupMembers.role, "owner"),
            ne(groupMembers.userId, userId),
          ),
        );

      if (otherOwnerCount === 0) {
        return NextResponse.json(
          {
            error:
              "Cannot demote the last owner. Use POST /api/groups/:slug/transfer-ownership to assign a new owner first.",
          },
          { status: 400 }
        );
      }
    }

    const [updated] = await db
      .update(groupMembers)
      .set({ role: nextRole as GroupRole })
      .where(and(eq(groupMembers.groupId, group.id), eq(groupMembers.userId, userId)))
      .returning({
        id: groupMembers.id,
        userId: groupMembers.userId,
        role: groupMembers.role,
      });

    if (!updated) {
      return NextResponse.json({ error: "Member not found" }, { status: 404 });
    }

    try {
      await revalidateGroupCaches(group.id, group.slug);
    } catch (cacheError) {
      console.error("Update group member role cache invalidation failed:", cacheError);
    }

    return NextResponse.json(updated);
  } catch (error) {
    console.error("Update group member role error:", error);
    return NextResponse.json({ error: "Failed to update role" }, { status: 500 });
  }
}
