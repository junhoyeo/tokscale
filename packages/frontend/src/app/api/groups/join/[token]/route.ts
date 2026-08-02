import { NextResponse } from "next/server";
import { getSessionFromRequest } from "@/lib/auth/requestSession";
import { revalidateGroupCaches } from "@/lib/groups/cache";
import {
  acceptGroupInvite,
  getGroupInvitePreview,
  GroupInviteError,
} from "@/lib/groups/invites";

const BROWSER_SESSION_OPTIONS = { allowAuthorizationHeader: false } as const;

interface RouteParams {
  params: Promise<{ token: string }>;
}

function errorResponse(error: GroupInviteError) {
  if (error.code === "not_found") {
    return NextResponse.json({ error: error.message }, { status: 404 });
  }

  return NextResponse.json({ error: error.message }, { status: error.code === "invalid" ? 400 : 403 });
}

export async function GET(_request: Request, { params }: RouteParams) {
  try {
    const { token } = await params;
    return NextResponse.json(await getGroupInvitePreview(token));
  } catch (error) {
    if (error instanceof GroupInviteError) {
      return errorResponse(error);
    }
    console.error("Get group invite preview error:", error);
    return NextResponse.json({ error: "Failed to fetch invite" }, { status: 500 });
  }
}

export async function POST(request: Request, { params }: RouteParams) {
  try {
    const session = await getSessionFromRequest(request, BROWSER_SESSION_OPTIONS);
    if (!session) {
      return NextResponse.json({ error: "Not authenticated" }, { status: 401 });
    }

    const { token } = await params;
    const accepted = await acceptGroupInvite(token, session);
    try {
      await revalidateGroupCaches(accepted.group.id, accepted.group.slug);
    } catch (cacheError) {
      console.error("Accept group invite cache invalidation failed:", cacheError);
    }

    return NextResponse.json(accepted);
  } catch (error) {
    if (error instanceof GroupInviteError) {
      return errorResponse(error);
    }
    console.error("Accept group invite error:", error);
    return NextResponse.json({ error: "Failed to accept invite" }, { status: 500 });
  }
}
