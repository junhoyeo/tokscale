import { NextResponse } from "next/server";
import { isAdmin } from "@/lib/auth/admin";
import { getSessionFromRequest } from "@/lib/auth/requestSession";
import type { SessionUser } from "@/lib/auth/session";

/**
 * Resolves the caller and confirms they may moderate.
 *
 * Returns 404 rather than 401/403 for everyone who is not an admin. The
 * endpoints and the review page are not something users should be able to
 * probe for: a 403 confirms the route exists, which invites attempts against
 * it. To a non-admin the moderation surface simply does not exist.
 *
 * `allowAuthorizationHeader: false` rejects personal API tokens outright.
 * Those sessions carry no githubId and so could never pass isAdmin anyway, but
 * failing here keeps the reason obvious instead of surfacing as a puzzling 404
 * for someone holding a valid token.
 */
export async function requireAdminSession(
  request: Request
): Promise<{ session: SessionUser } | { response: NextResponse }> {
  const session = await getSessionFromRequest(request, {
    allowAuthorizationHeader: false,
  });

  if (!isAdmin(session)) {
    return { response: NextResponse.json({ error: "Not found" }, { status: 404 }) };
  }

  return { session: session as SessionUser };
}
