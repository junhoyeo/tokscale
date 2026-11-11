import { after } from "next/server";

type ServerAnalyticsEvent =
  | "signed_up"
  | "cli_authorized"
  | "first_usage_submitted"
  | "group_created";

/**
 * Posts a custom event to Vercel Web Analytics the way `track` from
 * `@vercel/analytics/server` does, but with the event URL fixed to the route
 * path. The SDK takes that URL from the platform request context and cannot be
 * told otherwise: on the OAuth callback it would carry `code` and `state`. The
 * SDK also forwards the request's cookies, which the endpoint does not need.
 */
export function trackServerEvent(name: ServerAnalyticsEvent, request: Request): void {
  if (!process.env.VERCEL) {
    return;
  }

  const { origin, pathname } = new URL(request.url);
  const headers = {
    "content-type": "application/json",
    "user-agent": request.headers.get("user-agent") ?? "",
    "x-vercel-ip": request.headers.get("x-forwarded-for") ?? "",
    "x-va-server": "1",
  };

  try {
    after(async () => {
      try {
        await fetch(`${origin}/_vercel/insights/event`, {
          method: "POST",
          headers,
          body: JSON.stringify({ o: `${origin}${pathname}`, ts: Date.now(), r: "", en: name }),
        });
      } catch {
        // Analytics must not affect the response.
      }
    });
  } catch {
    // `after` can throw when called outside a request scope.
  }
}
