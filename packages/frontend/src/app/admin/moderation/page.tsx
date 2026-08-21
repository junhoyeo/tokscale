import type { Metadata } from "next";
import { notFound } from "next/navigation";
import { isAdmin } from "@/lib/auth/admin";
import { getSession } from "@/lib/auth/session";
import ModerationClient from "./ModerationClient";

// Never indexable, and not something a non-admin should be able to detect.
export const metadata: Metadata = {
  title: "Moderation | Tokscale",
  robots: { index: false, follow: false },
};

/**
 * Gated here rather than in middleware: src/middleware.ts deliberately makes no
 * database call, so it cannot resolve who the session belongs to.
 *
 * notFound() rather than a 403 — a forbidden response confirms the page exists.
 */
export default async function ModerationPage() {
  const session = await getSession();

  if (!isAdmin(session)) {
    notFound();
  }

  return <ModerationClient />;
}
