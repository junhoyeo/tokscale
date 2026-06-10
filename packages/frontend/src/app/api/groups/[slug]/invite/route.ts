import { NextResponse } from "next/server";
import { getSessionFromRequest } from "@/lib/auth/requestSession";
import { revalidateGroupCaches } from "@/lib/groups/cache";
import { createGroupInvite, GroupInviteError } from "@/lib/groups/invites";
import { getGroupMembership } from "@/lib/groups/permissions";
import { getGroupBySlug } from "@/lib/groups/queries";
import { canManageGroupRole, isGroupRole } from "@/lib/groups/utils";
import type { GroupRole } from "@/lib/db";

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
    if (!group.isPublic && !membership) {
      return NextResponse.json({ error: "Group not found" }, { status: 404 });
    }

    if (!membership || membership.role === "member") {
      return NextResponse.json({ error: "Forbidden" }, { status: 403 });
    }

    let body: Record<string, unknown> = {};
    try {
      const text = await request.text();
      if (text.length === 0) {
        body = {};
      } else {
        body = JSON.parse(text);
      }
    } catch {
      return NextResponse.json({ error: "Invalid JSON body" }, { status: 400 });
    }

    if (!body || typeof body !== "object" || Array.isArray(body)) {
      return NextResponse.json({ error: "Invalid JSON body" }, { status: 400 });
    }

    const role = isGroupRole(body.role) ? body.role : "member";
    const invitedUsername = typeof body.invitedUsername === "string"
      ? body.invitedUsername
      : null;

    if (!canManageGroupRole(membership.role, role)) {
      return NextResponse.json({ error: "Forbidden" }, { status: 403 });
    }

    const invite = await createGroupInvite({
      groupId: group.id,
      invitedBy: session.id,
      role: role as GroupRole,
      invitedUsername,
    });

    try {
      await revalidateGroupCaches(group.id, group.slug);
    } catch (cacheError) {
      console.error("Create group invite cache invalidation failed:", cacheError);
    }

    return NextResponse.json(
      {
        invite,
        joinUrl: `/groups/join/${invite.token}`,
      },
      { status: 201, headers: { "Cache-Control": "no-store, private" } }
    );
  } catch (error) {
    if (error instanceof GroupInviteError) {
      return NextResponse.json({ error: error.message }, { status: error.code === "invalid" ? 400 : 403 });
    }
    console.error("Create group invite error:", error);
    return NextResponse.json({ error: "Failed to create invite" }, { status: 500 });
  }
}
