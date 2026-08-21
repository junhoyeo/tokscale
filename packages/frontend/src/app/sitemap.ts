import type { MetadataRoute } from "next";
import { unstable_cache } from "next/cache";
import { desc, eq } from "drizzle-orm";
import { db, groups, submissions, users } from "@/lib/db";

// Metadata routes do not inherit the root layout's runtime rendering mode.
// Keep APP_URL runtime-configurable for reusable self-hosted images.
export const dynamic = "force-dynamic";

import {
  SITEMAP_GROUP_LIMIT,
  SITEMAP_USER_LIMIT,
  buildCoreEntries,
  buildGroupEntries,
  buildUserEntries,
  latestSubmissionTime,
  type SitemapGroupRow,
  type SitemapUserRow,
} from "@/lib/seo/sitemap";

/**
 * Profiles worth indexing, ranked so that truncation at SITEMAP_USER_LIMIT
 * keeps the most substantial ones.
 *
 * The INNER JOIN is the filter that matters: `submissions` holds exactly one
 * row per user (submissions_user_id_unique), so joining it drops accounts that
 * signed up via GitHub but never ran the CLI. Those render an empty profile,
 * and a sitemap full of empty pages is the fastest way to get a site flagged
 * for thin content.
 */
const loadUserRows = unstable_cache(
  async (): Promise<SitemapUserRow[]> =>
    db
    .select({ username: users.username, updatedAt: submissions.updatedAt })
    .from(users)
    .innerJoin(submissions, eq(submissions.userId, users.id))
    .orderBy(desc(submissions.totalTokens))
    .limit(SITEMAP_USER_LIMIT),
  ["sitemap-users"],
  { revalidate: 3600, tags: ["sitemap"] },
);

/** Private groups 404 for non-members, so only public ones can be listed. */
const loadGroupRows = unstable_cache(
  async (): Promise<SitemapGroupRow[]> =>
    db
    .select({ slug: groups.slug, updatedAt: groups.updatedAt })
    .from(groups)
    .where(eq(groups.isPublic, true))
    .orderBy(desc(groups.updatedAt))
    .limit(SITEMAP_GROUP_LIMIT),
  ["sitemap-groups"],
  { revalidate: 3600, tags: ["sitemap"] },
);

/**
 * A sitemap is an optimization, never a hard dependency. The route renders at
 * request time so its origin follows runtime APP_URL, while its database rows
 * are cached hourly. An unreachable DB therefore degrades to core pages.
 */
async function loadOrEmpty<T>(
  label: string,
  load: () => Promise<T[]>
): Promise<T[]> {
  try {
    return await load();
  } catch (error) {
    console.error(`[sitemap] failed to load ${label}:`, error);
    return [];
  }
}

export default async function sitemap(): Promise<MetadataRoute.Sitemap> {
  const now = new Date();

  const [userRows, groupRows] = await Promise.all([
    loadOrEmpty("users", loadUserRows),
    loadOrEmpty("groups", loadGroupRows),
  ]);

  // Anchored to real submission activity rather than `now`: the row loaders
  // revalidate hourly, so `now` would advance the core pages' lastmod every
  // hour whether or not anything changed. Falls back to `now` only when the
  // DB is unreachable and there is nothing truthful to report.
  const coreLastModified = latestSubmissionTime(userRows) ?? now;

  return [
    ...buildCoreEntries(coreLastModified),
    ...buildUserEntries(userRows, now),
    ...buildGroupEntries(groupRows, now),
  ];
}
