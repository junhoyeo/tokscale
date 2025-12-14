import { cookies } from "next/headers";
import { db, sessions, users, apiTokens } from "@/lib/db";
import { eq, and, gt, or, isNull } from "drizzle-orm";
import { generateRandomString } from "./utils";

const SESSION_COOKIE_NAME = "tt_session";
const SESSION_DURATION_MS = 30 * 24 * 60 * 60 * 1000; // 30 days

export interface SessionUser {
  id: string;
  username: string;
  displayName: string | null;
  avatarUrl: string | null;
  isAdmin: boolean;
}

/**
 * Get current session from request cookies.
 * Returns null if no valid session exists.
 */
export async function getSession(): Promise<SessionUser | null> {
  const cookieStore = await cookies();
  const sessionToken = cookieStore.get(SESSION_COOKIE_NAME)?.value;

  if (!sessionToken) {
    return null;
  }

  const result = await db
    .select({
      session: sessions,
      user: users,
    })
    .from(sessions)
    .innerJoin(users, eq(sessions.userId, users.id))
    .where(and(eq(sessions.token, sessionToken), gt(sessions.expiresAt, new Date())))
    .limit(1);

  if (result.length === 0) {
    return null;
  }

  const { user } = result[0];

  return {
    id: user.id,
    username: user.username,
    displayName: user.displayName,
    avatarUrl: user.avatarUrl,
    isAdmin: user.isAdmin,
  };
}

/**
 * Create a new session for a user.
 */
export async function createSession(
  userId: string,
  options: { source?: "web" | "cli"; userAgent?: string } = {}
): Promise<string> {
  const token = generateRandomString(64);
  const expiresAt = new Date(Date.now() + SESSION_DURATION_MS);

  await db.insert(sessions).values({
    userId,
    token,
    expiresAt,
    source: options.source ?? "web",
    userAgent: options.userAgent,
  });

  return token;
}

/**
 * Set session cookie.
 */
export async function setSessionCookie(token: string): Promise<void> {
  const cookieStore = await cookies();

  cookieStore.set(SESSION_COOKIE_NAME, token, {
    httpOnly: true,
    secure: process.env.NODE_ENV === "production",
    sameSite: "lax",
    maxAge: SESSION_DURATION_MS / 1000,
    path: "/",
  });
}

/**
 * Clear session cookie and delete session from database.
 */
export async function clearSession(): Promise<void> {
  const cookieStore = await cookies();
  const sessionToken = cookieStore.get(SESSION_COOKIE_NAME)?.value;

  if (sessionToken) {
    await db.delete(sessions).where(eq(sessions.token, sessionToken));
  }

  cookieStore.delete(SESSION_COOKIE_NAME);
}

/**
 * Validate API token for CLI requests.
 * Returns the user if token is valid, null otherwise.
 */
export async function validateApiToken(
  token: string
): Promise<SessionUser | null> {
  if (!token.startsWith("tt_")) {
    return null;
  }

  const result = await db
    .select({
      apiToken: apiTokens,
      user: users,
    })
    .from(apiTokens)
    .innerJoin(users, eq(apiTokens.userId, users.id))
    .where(
      and(
        eq(apiTokens.token, token),
        or(isNull(apiTokens.expiresAt), gt(apiTokens.expiresAt, new Date()))
      )
    )
    .limit(1);

  if (result.length === 0) {
    return null;
  }

  // Update last_used_at
  await db
    .update(apiTokens)
    .set({ lastUsedAt: new Date() })
    .where(eq(apiTokens.token, token));

  const { user } = result[0];

  return {
    id: user.id,
    username: user.username,
    displayName: user.displayName,
    avatarUrl: user.avatarUrl,
    isAdmin: user.isAdmin,
  };
}

/**
 * Get session from Authorization header (for API routes).
 */
export async function getSessionFromHeader(
  request: Request
): Promise<SessionUser | null> {
  const authHeader = request.headers.get("Authorization");

  if (!authHeader) {
    return null;
  }

  // Support "Bearer <token>" format
  const token = authHeader.startsWith("Bearer ")
    ? authHeader.slice(7)
    : authHeader;

  // Check if it's an API token (CLI)
  if (token.startsWith("tt_")) {
    return validateApiToken(token);
  }

  // Otherwise treat as session token (web)
  const result = await db
    .select({
      session: sessions,
      user: users,
    })
    .from(sessions)
    .innerJoin(users, eq(sessions.userId, users.id))
    .where(and(eq(sessions.token, token), gt(sessions.expiresAt, new Date())))
    .limit(1);

  if (result.length === 0) {
    return null;
  }

  const { user } = result[0];

  return {
    id: user.id,
    username: user.username,
    displayName: user.displayName,
    avatarUrl: user.avatarUrl,
    isAdmin: user.isAdmin,
  };
}
