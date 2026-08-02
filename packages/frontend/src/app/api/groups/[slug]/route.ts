import { revalidatePath } from "next/cache";
import { NextResponse } from "next/server";
import { eq } from "drizzle-orm";
import { db, groups } from "@/lib/db";
import { getSessionFromRequest } from "@/lib/auth/requestSession";
import { revalidateGroupCaches } from "@/lib/groups/cache";
import { getGroupMembership } from "@/lib/groups/permissions";
import { getGroupBySlug, getGroupMemberCount } from "@/lib/groups/queries";
import { generateUniqueGroupSlug } from "@/lib/groups/slugs";

const BROWSER_SESSION_OPTIONS = { allowAuthorizationHeader: false } as const;

interface RouteParams {
  params: Promise<{ slug: string }>;
}

function parseString(value: unknown): string | null {
  return typeof value === "string" ? value.trim() : null;
}

function parseNullableStringField(
  value: unknown,
  fieldName: string
): { value: string | null } | { response: NextResponse } {
  if (value === null) {
    return { value: null };
  }

  if (typeof value === "string") {
    return { value: value.trim() };
  }

  return {
    response: NextResponse.json(
      { error: `${fieldName} must be a string or null` },
      { status: 400 }
    ),
  };
}

function groupNotFoundResponse() {
  return NextResponse.json({ error: "Group not found" }, { status: 404 });
}

async function authorizeGroupRead(request: Request, slug: string) {
  const group = await getGroupBySlug(slug);
  if (!group) {
    return { response: groupNotFoundResponse() };
  }

  const session = await getSessionFromRequest(request);
  const membership = session ? await getGroupMembership(group.id, session.id) : null;

  if (!group.isPublic && !membership) {
    return { response: groupNotFoundResponse() };
  }

  return { group, session, membership };
}

export async function GET(request: Request, { params }: RouteParams) {
  try {
    const { slug } = await params;
    const authorized = await authorizeGroupRead(request, slug);

    if ("response" in authorized) {
      return authorized.response;
    }

    const memberCount = await getGroupMemberCount(authorized.group.id);
    return NextResponse.json({
      ...authorized.group,
      memberCount,
      membership: authorized.membership,
    });
  } catch (error) {
    console.error("Get group error:", error);
    return NextResponse.json({ error: "Failed to fetch group" }, { status: 500 });
  }
}

export async function PATCH(request: Request, { params }: RouteParams) {
  try {
    const session = await getSessionFromRequest(request, BROWSER_SESSION_OPTIONS);
    if (!session) {
      return NextResponse.json({ error: "Not authenticated" }, { status: 401 });
    }

    const { slug } = await params;
    const group = await getGroupBySlug(slug);
    if (!group) {
      return groupNotFoundResponse();
    }

    const membership = await getGroupMembership(group.id, session.id);
    if (!group.isPublic && !membership) {
      return groupNotFoundResponse();
    }
    if (!membership || membership.role === "member") {
      return NextResponse.json({ error: "Forbidden" }, { status: 403 });
    }

    const body = await request.json().catch(() => null);
    if (!body || typeof body !== "object" || Array.isArray(body)) {
      return NextResponse.json({ error: "Invalid JSON body" }, { status: 400 });
    }
    const b = body as Record<string, unknown>;

    const updateData: {
      name?: string;
      slug?: string;
      description?: string | null;
      isPublic?: boolean;
      avatarUrl?: string | null;
      updatedAt: Date;
    } = { updatedAt: new Date() };

    if (b.name !== undefined) {
      const name = parseString(b.name);
      if (!name) {
        return NextResponse.json({ error: "Group name cannot be empty" }, { status: 400 });
      }
      if (name.length > 100) {
        return NextResponse.json({ error: "Group name must be 100 characters or less" }, { status: 400 });
      }
      updateData.name = name;
      if (name !== group.name) {
        updateData.slug = await generateUniqueGroupSlug(name);
      }
    }

    if (b.description !== undefined) {
      const description = parseNullableStringField(b.description, "description");
      if ("response" in description) {
        return description.response;
      }
      updateData.description = description.value;
    }
    if (b.avatarUrl !== undefined) {
      const avatarUrl = parseNullableStringField(b.avatarUrl, "avatarUrl");
      if ("response" in avatarUrl) {
        return avatarUrl.response;
      }
      updateData.avatarUrl = avatarUrl.value;
    }
    if (typeof b.isPublic === "boolean") {
      updateData.isPublic = b.isPublic;
    }

    const [updated] = await db
      .update(groups)
      .set(updateData)
      .where(eq(groups.id, group.id))
      .returning();

    try {
      await revalidateGroupCaches(group.id, updated?.slug ?? group.slug);
    } catch (cacheError) {
      console.error("Update group cache invalidation failed:", cacheError);
    }

    if (updated?.slug && updated.slug !== group.slug) {
      try {
        revalidatePath(`/groups/${group.slug}`);
      } catch (cacheError) {
        console.error("Old group path revalidation failed:", cacheError);
      }
    }

    return NextResponse.json(updated);
  } catch (error) {
    console.error("Update group error:", error);
    return NextResponse.json({ error: "Failed to update group" }, { status: 500 });
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
      return groupNotFoundResponse();
    }

    const membership = await getGroupMembership(group.id, session.id);
    if (!group.isPublic && !membership) {
      return groupNotFoundResponse();
    }
    if (!membership || membership.role !== "owner") {
      return NextResponse.json({ error: "Only the owner can delete this group" }, { status: 403 });
    }

    const deleted = await db
      .delete(groups)
      .where(eq(groups.id, group.id))
      .returning({ id: groups.id });

    if (deleted.length === 0) {
      return NextResponse.json({ error: "Group not found" }, { status: 404 });
    }

    try {
      await revalidateGroupCaches(group.id, group.slug);
    } catch (cacheError) {
      console.error("Delete group cache invalidation failed:", cacheError);
    }

    return NextResponse.json({ success: true });
  } catch (error) {
    console.error("Delete group error:", error);
    return NextResponse.json({ error: "Failed to delete group" }, { status: 500 });
  }
}
