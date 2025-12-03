import { NextResponse } from "next/server";
import type { NextRequest } from "next/server";

// Routes that require authentication
const PROTECTED_ROUTES = ["/settings"];

export function middleware(request: NextRequest) {
  const { pathname } = request.nextUrl;

  // Check if route is protected
  const isProtected = PROTECTED_ROUTES.some((route) =>
    pathname.startsWith(route)
  );

  if (!isProtected) {
    return NextResponse.next();
  }

  // Get session cookie
  const sessionToken = request.cookies.get("tt_session")?.value;

  if (!sessionToken) {
    // Redirect to login
    const loginUrl = new URL("/api/auth/github", request.url);
    loginUrl.searchParams.set("returnTo", pathname);
    return NextResponse.redirect(loginUrl);
  }

  // Session exists, allow access
  // Note: We don't validate the session here to avoid database calls in middleware
  // The actual session validation happens in the page/API route
  return NextResponse.next();
}

export const config = {
  matcher: ["/settings/:path*"],
};
