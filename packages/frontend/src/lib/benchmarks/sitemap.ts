import type { MetadataRoute } from "next";
import { benchmarkData } from "./index";
import { benchmarksUrl, benchmarkUrl, modelsAllUrl } from "./urls";

/**
 * Sitemap entries for /benchmarks and each benchmark page with results.
 * lastModified is the date the snapshot was retrieved, never the request
 * time. Link-only benchmarks have no page of their own.
 */
export function benchmarksSitemapEntries(): MetadataRoute.Sitemap {
  const pages = benchmarkData.list().filter((b) => b.definition.status !== "link-only" && b.snapshot);
  const dates = pages.map((b) => new Date(`${b.snapshot?.retrievedAt.slice(0, 10)}T00:00:00.000Z`));
  const newest = new Date(Math.max(...dates.map((d) => d.getTime())));
  return [
    { url: benchmarksUrl(), lastModified: newest, changeFrequency: "weekly", priority: 0.6 },
    ...pages.map((b, i) => ({
      url: benchmarkUrl(b.definition.slug),
      lastModified: dates[i],
      changeFrequency: "weekly" as const,
      priority: b.definition.status === "history" ? 0.3 : 0.5,
    })),
  ];
}

/**
 * /models/all, the full catalog. `lastModified` should be the newest
 * submission in the rollups (Coverage.lastDataAt), or null to omit.
 */
export function modelsCatalogSitemapEntry(lastModified: Date | null): MetadataRoute.Sitemap[number] {
  return { url: modelsAllUrl(), changeFrequency: "daily", priority: 0.6, ...(lastModified ? { lastModified } : {}) };
}
