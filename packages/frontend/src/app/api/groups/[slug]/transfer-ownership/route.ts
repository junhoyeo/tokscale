import { NextResponse } from "next/server";
import { and, eq, ne } from "drizzle-orm";
import { db, groupMembers } from "@/lib/db";
import { getSessionFromRequest } from "@/lib/auth/requestSession";
import { revalidateGroupCaches } from "@/lib/groups/cache";
import { getGroupMembership } from "@/lib/groups/permissions";
import { getGroupBySlug } from "@/lib/groups/queries";
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

    const callerMembership = await getGroupMembership(group.id, session.id);
    if (!group.isPublic && !callerMembership) {
      return NextResponse.json({ error: "Group not found" }, { status: 404 });
    }

    if (!callerMembership || callerMembership.role !== "owner") {
      return NextResponse.json(
        { error: "Only the current owner can transfer ownership" },
        { status: 403 }
      );
    }

    const body = await request.json().catch(() => null);
    if (!body || typeof body !== "object" || Array.isArray(body)) {
      return NextResponse.json({ error: "Invalid JSON body" }, { status: 400 });
    }

    const { targetUserId } = body as Record<string, unknown>;
    if (typeof targetUserId !== "string" || !targetUserId) {
      return NextResponse.json(
        { error: "targetUserId is required" },
        { status: 400 }
      );
    }

    if (targetUserId === session.id) {
      return NextResponse.json(
        { error: "You are already the owner" },
        { status: 400 }
      );
    }

    const targetMembership = await getGroupMembership(group.id, targetUserId);
    if (!targetMembership) {
      return NextResponse.json(
        { error: "Target user is not a member of this group" },
        { status: 400 }
      );
    }

    if (targetMembership.role === "owner") {
      return NextResponse.json(
        { error: "Target user is already an owner" },
        { status: 400 }
      );
    }

    // Predicate-guarded updates inside the transaction prevent a race where
    // two concurrent POSTs from the same owner both pass the pre-check above
    // and both try to demote the caller / promote different targets. The
    // `role = 'owner'` guard on the caller and `role != 'owner'` guard on
    // the target ensure that whichever transaction commits second sees zero
    // matching rows and aborts, preserving the single-owner invariant.
    const TRANSFER_RACE = new Error("TRANSFER_RACE");

    try {
      const updated = await db.transaction(async (tx) => {
        const [callerRow] = await tx
          .update(groupMembers)
          .set({ role: "admin" as GroupRole })
          .where(
            and(
              eq(groupMembers.groupId, group.id),
              eq(groupMembers.userId, session.id),
              eq(groupMembers.role, "owner")
            )
          )
          .returning({ id: groupMembers.id, userId: groupMembers.userId, role: groupMembers.role });

        if (!callerRow) {
          throw TRANSFER_RACE;
        }

        const [targetRow] = await tx
          .update(groupMembers)
          .set({ role: "owner" as GroupRole })
          .where(
            and(
              eq(groupMembers.groupId, group.id),
              eq(groupMembers.userId, targetUserId),
              ne(groupMembers.role, "owner")
            )
          )
          .returning({ id: groupMembers.id, userId: groupMembers.userId, role: groupMembers.role });

        if (!targetRow) {
          throw TRANSFER_RACE;
        }

        return [callerRow, targetRow];
      });

      try {
        await revalidateGroupCaches(group.id, group.slug);
      } catch (cacheError) {
        console.error("Transfer ownership cache invalidation failed:", cacheError);
      }

      return NextResponse.json({ members: updated });
    } catch (txError) {
      if (txError === TRANSFER_RACE) {
        return NextResponse.json(
          { error: "Ownership state changed during transfer; please retry" },
          { status: 409 }
        );
      }
      throw txError;
    }
  } catch (error) {
    console.error("Transfer ownership error:", error);
    return NextResponse.json({ error: "Failed to transfer ownership" }, { status: 500 });
  }
}
