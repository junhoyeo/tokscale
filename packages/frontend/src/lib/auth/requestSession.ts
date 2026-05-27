import { getSession, getSessionFromHeader, type SessionUser } from "./session";

const MUTATING_METHODS = new Set(["POST", "PUT", "PATCH", "DELETE"]);

function getAllowedOrigins(): string[] {
  const env = process.env.CSRF_ALLOWED_ORIGINS;
  if (env) {
    return env.split(",").map((o) => o.trim()).filter(Boolean);
  }
  return ["https://tokscale.dev", "http://localhost:3000"];
}

export async function getSessionFromRequest(request: Request): Promise<SessionUser | null> {
  const authHeader = request.headers.get("Authorization");

  if (authHeader) {
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
