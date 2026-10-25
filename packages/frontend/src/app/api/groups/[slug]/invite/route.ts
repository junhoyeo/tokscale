import { NextResponse } from "next/server";
import { getSessionFromRequest } from "@/lib/auth/requestSession";
import { revalidateGroupCaches } from "@/lib/groups/cache";
import { createGroupInvite, listGroupInvites, revokeGroupInvite, GroupInviteError } from "@/lib/groups/invites";
import { getGroupMembership } from "@/lib/groups/permissions";
import { getGroupBySlug } from "@/lib/groups/queries";
import { canManageGroupRole, isGroupRole } from "@/lib/groups/utils";
import type { GroupRole } from "@/lib/db";

const BROWSER_SESSION_OPTIONS = { allowAuthorizationHeader: false } as const;

type RouteParams = { params: Promise<{ slug: string }> };

async function authorize(request: Request, params: RouteParams["params"]) {
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

  return { group, session, membership };
}

function inviteErrorResponse(error: unknown) {
  if (error instanceof GroupInviteError) {
    const status = { invalid: 400, forbidden: 403, not_found: 404, conflict: 409 }[error.code];
    return NextResponse.json({ error: error.message }, { status, headers: { "Cache-Control": "no-store, private" } });
  }
  console.error("Group invite request failed:", error);
  return NextResponse.json({ error: "Failed to manage invites" }, { status: 500 });
}

export async function GET(request: Request, { params }: RouteParams) {
  try {
    const authorized = await authorize(request, params);
    if (authorized instanceof NextResponse) return authorized;
    return NextResponse.json({ invites: await listGroupInvites(authorized.group.id, authorized.membership.role) },
      { headers: { "Cache-Control": "no-store, private" } });
  } catch (error) { return inviteErrorResponse(error); }
}

export async function DELETE(request: Request, { params }: RouteParams) {
  try {
    const authorized = await authorize(request, params);
    if (authorized instanceof NextResponse) return authorized;
    let body: unknown;
    try { body = await request.json(); } catch { throw new GroupInviteError("invalid", "Invalid JSON body"); }
    if (!body || typeof body !== "object" || !("inviteId" in body) || typeof body.inviteId !== "string" ||
      !/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(body.inviteId)) {
      throw new GroupInviteError("invalid", "Invalid invite ID");
    }
    await revokeGroupInvite(authorized.group.id, body.inviteId, authorized.session.id);
    return NextResponse.json({ revoked: true }, { headers: { "Cache-Control": "no-store, private" } });
  } catch (error) { return inviteErrorResponse(error); }
}

export async function POST(
  request: Request,
  { params }: { params: Promise<{ slug: string }> }
) {
  try {
    const authorized = await authorize(request, params);
    if (authorized instanceof NextResponse) return authorized;
    const { group, session, membership } = authorized;

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

    const expiresInDays = body.expiresInDays ?? 14;
    if (typeof expiresInDays !== "number" || !Number.isInteger(expiresInDays) || expiresInDays < 1 || expiresInDays > 30) {
      throw new GroupInviteError("invalid", "Invite expiry must be between 1 and 30 days");
    }
    const maxUses = body.maxUses ?? 1;
    if (typeof maxUses !== "number" || !Number.isInteger(maxUses) || maxUses < 1 || maxUses > 1000 || (invitedUsername && maxUses !== 1)) {
      throw new GroupInviteError("invalid", "Use 1–1000 joins, or one join for an account-specific invite");
    }
    const invite = await createGroupInvite({
      groupId: group.id,
      invitedBy: session.id,
      role: role as GroupRole,
      invitedUsername,
      maxUses,
      expiresAt: new Date(Date.now() + expiresInDays * 86400000),
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
  } catch (error) { return inviteErrorResponse(error); }
}
