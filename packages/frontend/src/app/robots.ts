import type { MetadataRoute } from "next";
import { getPublicOrigin } from "@/lib/seo/urls";

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
