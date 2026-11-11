import { track } from "@vercel/analytics/server";
import { after } from "next/server";

type ServerAnalyticsEvent =
  | "signed_up"
  | "cli_authorized"
  | "first_usage_submitted"
  | "group_created";

export function trackServerEvent(name: ServerAnalyticsEvent, request: Request): void {
  if (!process.env.VERCEL) {
    return;
  }

  // Left to itself the SDK forwards the request's cookies and takes the event URL
  // from its Referer, which on /device carries the user code. Pass only what counts
  // a visitor, and the bare origin in place of the Referer.
  const headers = {
    "user-agent": request.headers.get("user-agent") ?? undefined,
    "x-forwarded-for": request.headers.get("x-forwarded-for") ?? undefined,
    referer: new URL(request.url).origin,
  };

  try {
    after(async () => {
      try {
        await track(name, undefined, { headers });
      } catch {
        // Analytics must not affect the response.
      }
    });
  } catch {
    // `after` can throw when called outside a request scope.
  }
}
