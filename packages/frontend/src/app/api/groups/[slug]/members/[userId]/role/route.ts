import { NextResponse } from "next/server";
import { and, eq, ne } from "drizzle-orm";
import { db, groupMembers } from "@/lib/db";
import { getSessionFromRequest } from "@/lib/auth/requestSession";
import { revalidateGroupCaches } from "@/lib/groups/cache";
import { getGroupMembership } from "@/lib/groups/permissions";
import { getGroupBySlug } from "@/lib/groups/queries";
import { canManageGroupRole, isGroupRole } from "@/lib/groups/utils";
import type { GroupRole } from "@/lib/db";

const BROWSER_SESSION_OPTIONS = { allowAuthorizationHeader: false } as const;

interface RouteParams {
  params: Promise<{ slug: string; userId: string }>;
}

export async function PATCH(request: Request, { params }: RouteParams) {
  try {
    const session = await getSessionFromRequest(request, BROWSER_SESSION_OPTIONS);
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

    const membership = await getGroupMembership(group.id, session.id);
    if (!group.isPublic && !membership) {
      return NextResponse.json({ error: "Group not found" }, { status: 404 });
    }

    const updateResult = await db.transaction(async (tx) => {
      const lockedMembers = new Map<string, { role: GroupRole }>();
      const lockUserIds = Array.from(new Set([session.id, userId])).sort();

      for (const lockUserId of lockUserIds) {
        const [member] = await tx
          .select({ role: groupMembers.role })
          .from(groupMembers)
          .where(and(eq(groupMembers.groupId, group.id), eq(groupMembers.userId, lockUserId)))
          .limit(1)
          .for("update");

        if (member) {
          lockedMembers.set(lockUserId, member);
        }
      }

      const actor = lockedMembers.get(session.id);

      if (!actor) {
        return { status: "forbidden" as const };
      }

      const target = lockedMembers.get(userId);

      if (!target) {
        return { status: "not_found" as const };
      }

      if (target.role === "owner") {
        const otherOwners = await tx
          .select({ id: groupMembers.id })
          .from(groupMembers)
          .where(
            and(
              eq(groupMembers.groupId, group.id),
              eq(groupMembers.role, "owner"),
              ne(groupMembers.userId, userId)
            )
          )
          .for("update");

        return otherOwners.length === 0
          ? { status: "last_owner" as const }
          : { status: "forbidden" as const };
      }

      if (!canManageGroupRole(actor.role, target.role) || !canManageGroupRole(actor.role, nextRole)) {
        return { status: "forbidden" as const };
      }

      const [updated] = await tx
        .update(groupMembers)
        .set({ role: nextRole as GroupRole })
        .where(
          and(
            eq(groupMembers.groupId, group.id),
            eq(groupMembers.userId, userId),
            eq(groupMembers.role, target.role)
          )
        )
        .returning({
          id: groupMembers.id,
          userId: groupMembers.userId,
          role: groupMembers.role,
        });

      return updated
        ? { status: "updated" as const, member: updated }
        : { status: "not_found" as const };
    });

    if (updateResult.status === "forbidden") {
      return NextResponse.json({ error: "Forbidden" }, { status: 403 });
    }

    if (updateResult.status === "last_owner") {
      return NextResponse.json(
        {
          error:
            "Cannot demote the last owner. Use POST /api/groups/:slug/transfer-ownership to assign a new owner first.",
        },
        { status: 400 }
      );
    }

    if (updateResult.status === "not_found") {
      return NextResponse.json({ error: "Member not found" }, { status: 404 });
    }

    const updated = updateResult.member;

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
