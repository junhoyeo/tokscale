import type { MetadataRoute } from "next";
import {
  LEGAL_PATHS,
  groupUrl,
  homeUrl,
  leaderboardUrl,
  legalUrl,
  profileUrl,
} from "./urls";

/**
 * When the legal pages' text last changed. Hardcoded on purpose: these pages
 * are static, so their lastmod must be a real edit date rather than anything
 * derived from request or build time — the same rule that applies to the
 * data-driven entries below.
 *
 * Bump this, and the "Last updated" line rendered on each page, together.
 */
const LEGAL_LAST_MODIFIED = new Date("2026-07-30T00:00:00.000Z");

/**
 * sitemaps.org caps one sitemap file at 50,000 URLs / 50 MB uncompressed, and
 * an over-limit file is rejected wholesale rather than truncated. We budget
 * each section separately so a runaway section can't invalidate the sitemap.
 *
 * If a section ever actually saturates its budget, split into per-segment
 * sitemaps (app/u/sitemap.ts, app/groups/sitemap.ts) instead of raising these.
 * Next.js serves `generateSitemaps()` shards at /u/sitemap/0.xml etc. without
 * emitting an index file, so sharding also means listing every shard in
 * robots.ts by hand — nested per-segment sitemaps stay far simpler.
 */
export const SITEMAP_USER_LIMIT = 40_000;
export const SITEMAP_GROUP_LIMIT = 5_000;

export interface SitemapUserRow {
  /** Canonical casing from the DB — /u/[username] permanent-redirects other
   *  casings, and a sitemap should never point at a redirect. */
  username: string;
  /** Last submission time; null for rows that predate the column default. */
  updatedAt: Date | null;
}

export interface SitemapGroupRow {
  slug: string;
  updatedAt: Date | null;
}

/**
 * The newest submission time across the listed users, which is the moment the
 * leaderboard (and the top-5 table on the home page) last actually changed.
 *
 * Used as the core pages' `lastmod` instead of the generation timestamp.
 * Google only honors lastmod when it is "consistently and verifiably accurate"
 * — a value that advances on every hourly revalidation while the page content
 * is unchanged is precisely the pattern that teaches it to ignore ours, and it
 * discounts lastmod per-site rather than per-URL.
 *
 * Returns null for an empty list so the caller can fall back explicitly.
 */
export function latestSubmissionTime(
  rows: readonly SitemapUserRow[]
): Date | null {
  let latest: Date | null = null;

  for (const row of rows) {
    if (row.updatedAt && (latest === null || row.updatedAt > latest)) {
      latest = row.updatedAt;
    }
  }

  return latest;
}

/**
 * Pages that exist independently of the database.
 *
 * `lastModified` should come from latestSubmissionTime(), not `new Date()`.
 *
 * Deliberately excluded, and mirrored by the disallow list in app/robots.ts:
 * - /settings, /profile, /device  — auth-gated or redirect-only
 * - /groups/new                   — auth-gated
 * - /groups/join/[token]          — invite tokens, must never be indexed
 * - /groups                       — redirects to /leaderboard?view=groups
 * - /local                        — client-only viewer; renders empty to a
 *                                   crawler, so listing it would just add a
 *                                   thin-content URL to the index
 *
 * The leaderboard is listed once, bare. Its filter params (period, sortBy,
 * page, from/to, search) all canonicalize back to this URL, so listing any of
 * them would point crawlers at pages that disclaim themselves.
 */
export function buildCoreEntries(lastModified: Date): MetadataRoute.Sitemap {
  // changeFrequency and priority are inert for Google, which documents that it
  // ignores both. Kept because Bing, Naver and Daum still read them and they
  // cost nothing.
  return [
    {
      url: homeUrl(),
      lastModified,
      changeFrequency: "daily",
      priority: 1,
    },
    {
      url: leaderboardUrl(),
      lastModified,
      changeFrequency: "hourly",
      priority: 0.9,
    },
    {
      url: leaderboardUrl("groups"),
      lastModified,
      changeFrequency: "daily",
      priority: 0.7,
    },
    // Low priority but deliberately listed: ad networks and search engines
    // both check that a reachable privacy policy exists.
    ...LEGAL_PATHS.map((page) => ({
      url: legalUrl(page),
      lastModified: LEGAL_LAST_MODIFIED,
      changeFrequency: "yearly" as const,
      priority: 0.3,
    })),
  ];
}

export function buildUserEntries(
  rows: readonly SitemapUserRow[],
  fallbackLastModified: Date
): MetadataRoute.Sitemap {
  return rows.map((row) => ({
    url: profileUrl(row.username),
    lastModified: row.updatedAt ?? fallbackLastModified,
    changeFrequency: "daily",
    priority: 0.7,
  }));
}

export function buildGroupEntries(
  rows: readonly SitemapGroupRow[],
  fallbackLastModified: Date
): MetadataRoute.Sitemap {
  return rows.map((row) => ({
    url: groupUrl(row.slug),
    lastModified: row.updatedAt ?? fallbackLastModified,
    changeFrequency: "weekly",
    priority: 0.6,
  }));
}
