import type { MetadataRoute } from "next";
import { getPublicOrigin } from "@/lib/seo/urls";

// Metadata routes are separate entry points from the root layout. They must
// evaluate APP_URL at request time so one image can serve each deployment's
// public origin.
export const dynamic = "force-dynamic";

export default function robots(): MetadataRoute.Robots {
  return {
    rules: {
      userAgent: "*",
      allow: "/",
      // Mirrors the exclusions documented on buildCoreEntries(). /groups/join/
      // is the one that actually matters: those URLs carry single-use invite
      // tokens and must never reach an index.
      disallow: [
        "/api/",
        "/admin",
        "/settings",
        "/profile",
        "/device",
        "/groups/new",
        "/groups/join/",
      ],
    },
    sitemap: `${getPublicOrigin()}/sitemap.xml`,
  };
}
