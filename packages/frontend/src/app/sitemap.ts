import type { MetadataRoute } from "next";
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
async function loadUserRows(): Promise<SitemapUserRow[]> {
  return db
    .select({ username: users.username, updatedAt: submissions.updatedAt })
    .from(users)
    .innerJoin(submissions, eq(submissions.userId, users.id))
    .orderBy(desc(submissions.totalTokens))
    .limit(SITEMAP_USER_LIMIT);
}

/** Private groups 404 for non-members, so only public ones can be listed. */
async function loadGroupRows(): Promise<SitemapGroupRow[]> {
  return db
    .select({ slug: groups.slug, updatedAt: groups.updatedAt })
    .from(groups)
    .where(eq(groups.isPublic, true))
    .orderBy(desc(groups.updatedAt))
    .limit(SITEMAP_GROUP_LIMIT);
}

/**
 * A sitemap is an optimization, never a hard dependency. This route runs during
 * `next build`, so an unreachable DB here would otherwise fail the deploy —
 * degrade to the core pages instead.
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

  // Anchored to real submission activity rather than `now`: this route
  // revalidates hourly, so `now` would advance the core pages' lastmod every
  // hour whether or not anything changed, which is what makes Google stop
  // trusting the field. Falls back to `now` only when the DB is unreachable
  // and there is nothing truthful to report.
  const coreLastModified = latestSubmissionTime(userRows) ?? now;

  return [
    ...buildCoreEntries(coreLastModified),
    ...buildUserEntries(userRows, now),
    ...buildGroupEntries(groupRows, now),
  ];
}
