import { NextResponse } from "next/server";
import { cookies } from "next/headers";
import {
  exchangeCodeForToken,
  getGitHubUser,
  getGitHubUserEmail,
} from "@/lib/auth/github";
import { createSession, setSessionCookie } from "@/lib/auth/session";
import { db, users } from "@/lib/db";
import { eq } from "drizzle-orm";

export async function GET(request: Request) {
  const { searchParams } = new URL(request.url);
  const code = searchParams.get("code");
  const state = searchParams.get("state");
  const error = searchParams.get("error");

  const baseUrl = process.env.NEXT_PUBLIC_URL || "http://localhost:3000";

  // Handle OAuth errors
  if (error) {
    console.error("GitHub OAuth error:", error);
    return NextResponse.redirect(`${baseUrl}/?error=oauth_error`);
  }

  if (!code || !state) {
    return NextResponse.redirect(`${baseUrl}/?error=missing_params`);
  }

  // Validate CSRF state
  const cookieStore = await cookies();
  const storedStateRaw = cookieStore.get("oauth_state")?.value;

  if (!storedStateRaw) {
    return NextResponse.redirect(`${baseUrl}/?error=invalid_state`);
  }

  let storedState: { state: string; returnTo: string };
  try {
    storedState = JSON.parse(storedStateRaw);
  } catch {
    return NextResponse.redirect(`${baseUrl}/?error=invalid_state`);
  }

  if (storedState.state !== state) {
    return NextResponse.redirect(`${baseUrl}/?error=state_mismatch`);
  }

  // Clear the state cookie
  cookieStore.delete("oauth_state");

  try {
    // Exchange code for access token
    const accessToken = await exchangeCodeForToken(code);

    // Fetch user info from GitHub
    const githubUser = await getGitHubUser(accessToken);
    const email = githubUser.email || (await getGitHubUserEmail(accessToken));

    // Upsert user in database
    const existingUser = await db
      .select()
      .from(users)
      .where(eq(users.githubId, githubUser.id))
      .limit(1);

    let userId: string;

    if (existingUser.length > 0) {
      // Update existing user
      userId = existingUser[0].id;
      await db
        .update(users)
        .set({
          username: githubUser.login,
          displayName: githubUser.name,
          avatarUrl: githubUser.avatar_url,
          email: email,
          updatedAt: new Date(),
        })
        .where(eq(users.id, userId));
    } else {
      // Create new user
      const [newUser] = await db
        .insert(users)
        .values({
          githubId: githubUser.id,
          username: githubUser.login,
          displayName: githubUser.name,
          avatarUrl: githubUser.avatar_url,
          email: email,
        })
        .returning({ id: users.id });

      userId = newUser.id;
    }

    // Create session
    const sessionToken = await createSession(userId, {
      source: "web",
      userAgent: request.headers.get("user-agent") || undefined,
    });

    // Set session cookie
    await setSessionCookie(sessionToken);

    // Redirect to return URL
    const returnTo = storedState.returnTo || "/";
    return NextResponse.redirect(`${baseUrl}${returnTo}`);
  } catch (err) {
    console.error("GitHub OAuth callback error:", err);
    return NextResponse.redirect(`${baseUrl}/?error=auth_failed`);
  }
}
