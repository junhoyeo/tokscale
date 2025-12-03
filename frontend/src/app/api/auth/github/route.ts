import { NextResponse } from "next/server";
import { cookies } from "next/headers";
import { getAuthorizationUrl } from "@/lib/auth/github";
import { generateRandomString } from "@/lib/auth/utils";

export async function GET(request: Request) {
  const { searchParams } = new URL(request.url);
  const returnTo = searchParams.get("returnTo") || "/";

  // Generate CSRF state
  const state = generateRandomString(32);

  // Store state and returnTo in cookie
  const cookieStore = await cookies();
  cookieStore.set(
    "oauth_state",
    JSON.stringify({ state, returnTo }),
    {
      httpOnly: true,
      secure: process.env.NODE_ENV === "production",
      sameSite: "lax",
      maxAge: 60 * 10, // 10 minutes
      path: "/",
    }
  );

  // Redirect to GitHub
  return NextResponse.redirect(getAuthorizationUrl(state));
}
