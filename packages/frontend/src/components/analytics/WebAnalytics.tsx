"use client";

import { Analytics, type BeforeSendEvent } from "@vercel/analytics/next";
import { redactAnalyticsUrl } from "@/lib/analytics/redact";

function redactEvent(event: BeforeSendEvent): BeforeSendEvent {
  return { ...event, url: redactAnalyticsUrl(event.url) };
}

export function WebAnalytics() {
  return <Analytics beforeSend={redactEvent} />;
}
