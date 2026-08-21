import { getSession, getSessionFromHeader, type SessionUser } from "./session";
import { getConfiguredPublicOrigin, getPublicOrigin } from "@/lib/seo/urls";

const MUTATING_METHODS = new Set(["POST", "PUT", "PATCH", "DELETE"]);

interface GetSessionFromRequestOptions {
  allowAuthorizationHeader?: boolean;
}

function getAllowedOrigins(): string[] {
  const env = process.env.CSRF_ALLOWED_ORIGINS;
  const origins = env
    ? env.split(",").map((o) => o.trim()).filter(Boolean)
    : [getPublicOrigin(), "http://localhost:3000"];

  // An explicit allowlist should not be widened by the hosted fallback. Add
  // only an explicitly configured valid APP_URL in that mode.
  const configuredOrigin = getConfiguredPublicOrigin();
  if (configuredOrigin && !origins.includes(configuredOrigin)) {
    origins.push(configuredOrigin);
  }

  return origins;
}

export async function getSessionFromRequest(
  request: Request,
  options: GetSessionFromRequestOptions = {}
): Promise<SessionUser | null> {
  const authHeader = request.headers.get("Authorization");

  if (authHeader && options.allowAuthorizationHeader !== false) {
    return getSessionFromHeader(request);
  }

  if (MUTATING_METHODS.has(request.method)) {
    // Cookie-authenticated mutations must carry an Origin header that
    // matches the allowlist. A missing Origin header is also rejected:
    // modern browsers always set Origin on cross-origin mutating
    // requests, so a missing value typically means a non-browser client
    // that should be using a Bearer token instead.
    const origin = request.headers.get("Origin");
    const allowed = getAllowedOrigins();
    if (!origin || !allowed.includes(origin)) {
      return null;
    }
  }

  return getSession();
}
