import { NextResponse } from "next/server";
import { and, eq } from "drizzle-orm";
import { db, groupMembers } from "@/lib/db";
import { getSessionFromRequest } from "@/lib/auth/requestSession";
import { revalidateGroupCaches } from "@/lib/groups/cache";
import { getGroupMembership } from "@/lib/groups/permissions";
import { getGroupBySlug } from "@/lib/groups/queries";

const BROWSER_SESSION_OPTIONS = { allowAuthorizationHeader: false } as const;

export async function POST(
  request: Request,
  { params }: { params: Promise<{ slug: string }> }
) {
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

    const membership = await getGroupMembership(group.id, session.id);
    if (!membership) {
      return NextResponse.json({ error: "Not a member of this group" }, { status: 404 });
    }

    if (membership.role === "owner") {
      return NextResponse.json(
        { error: "Owners must transfer ownership (POST /api/groups/:slug/transfer-ownership) or delete the group before leaving" },
        { status: 400 }
      );
    }

    await db
      .delete(groupMembers)
      .where(and(eq(groupMembers.groupId, group.id), eq(groupMembers.userId, session.id)));

    try {
      await revalidateGroupCaches(group.id, group.slug);
    } catch (cacheError) {
      console.error("Leave group cache invalidation failed:", cacheError);
    }

    return NextResponse.json({ success: true });
  } catch (error) {
    console.error("Leave group error:", error);
    return NextResponse.json({ error: "Failed to leave group" }, { status: 500 });
  }
}
