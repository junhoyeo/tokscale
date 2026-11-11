/**
 * Canonical URLs and sitemap entries for the explore surfaces: the Models and
 * Agents leaderboard views, the /models catalog, /models/[key] and
 * /methodology.
 *
 * Built on getPublicOrigin() from ./urls so canonicals and the sitemap agree
 * (see the note at the top of urls.ts).
 */
import type { MetadataRoute } from "next";
import { getPublicOrigin } from "./urls";

/**
 * Models and Agents are separate rankings, so each canonicalizes to its own
 * view URL. period and sort are filters over the same ranking and collapse.
 */
export function exploreLeaderboardUrl(view: "models" | "agents"): string {
  return `${getPublicOrigin()}/leaderboard?view=${view}`;
}

export function modelsIndexUrl(): string {
  return `${getPublicOrigin()}/models`;
}

/** /models/[key] accepts ?period=; every period is the same page, so the bare URL is canonical. */
export function modelUrl(key: string): string {
  return `${getPublicOrigin()}/models/${encodeURIComponent(key)}`;
}

export function methodologyUrl(): string {
  return `${getPublicOrigin()}/methodology`;
}

/** Company pages live inside Capital (see @/lib/capital/urls). */
export { companyPath } from "@/lib/capital/urls";

export function modelPath(key: string): string {
  return `/models/${encodeURIComponent(key)}`;
}

export function profilePath(username: string): string {
  return `/u/${encodeURIComponent(username)}`;
}

/**
 * When /methodology text last changed. Static content gets a real edit date,
 * never a request or build time.
 */
export const METHODOLOGY_LAST_MODIFIED = new Date("2026-10-19T00:00:00.000Z");

/**
 * Sitemap entries for the explore pages. `lastModified` should be the newest
 * submission included in the rollups (Coverage.lastDataAt), or null to omit.
 */
export function buildExploreSitemapEntries(
  modelKeys: readonly string[],
  lastModified: Date | null,
): MetadataRoute.Sitemap {
  const dated = lastModified ? { lastModified } : {};
  return [
    { url: exploreLeaderboardUrl("models"), changeFrequency: "daily", priority: 0.8, ...dated },
    { url: exploreLeaderboardUrl("agents"), changeFrequency: "daily", priority: 0.7, ...dated },
    { url: modelsIndexUrl(), changeFrequency: "daily", priority: 0.8, ...dated },
    { url: methodologyUrl(), changeFrequency: "monthly", priority: 0.4, lastModified: METHODOLOGY_LAST_MODIFIED },
    ...modelKeys.map((key) => ({
      url: modelUrl(key),
      changeFrequency: "daily" as const,
      priority: 0.6,
      ...dated,
    })),
  ];
}

export interface BreadcrumbItem {
  name: string;
  url: string;
}

/** schema.org BreadcrumbList for a JSON-LD script tag. */
export function breadcrumbJsonLd(items: readonly BreadcrumbItem[]) {
  return {
    "@context": "https://schema.org",
    "@type": "BreadcrumbList",
    itemListElement: items.map((item, index) => ({
      "@type": "ListItem",
      position: index + 1,
      name: item.name,
      item: item.url,
    })),
  };
}

/**
 * Serialise JSON-LD for dangerouslySetInnerHTML. "<" is escaped so a model
 * name can never close the script tag.
 */
export function serializeJsonLd(value: unknown): string {
  return JSON.stringify(value).replace(/</g, "\\u003c");
}
