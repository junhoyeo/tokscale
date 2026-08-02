/**
 * Canonical URL builders for every indexable page.
 *
 * Both the sitemap and each page's <link rel="canonical"> are built from these,
 * because the two disagreeing is worse than either being absent: a sitemap URL
 * that doesn't match the page's own canonical is a conflicting signal, and the
 * page gets dropped rather than arbitrated.
 *
 * Must stay in sync with `metadataBase` in app/layout.tsx.
 */
export const SITE_URL = "https://tokscale.ai";

/**
 * The bare origin, with no trailing slash, which is what both consumers emit
 * verbatim: prod serves `<loc>https://tokscale.ai</loc>` in the sitemap and
 * `rel="canonical" href="https://tokscale.ai"` on the page.
 *
 * Don't add a trailing slash back "for correctness" — it would desync the two.
 * (A `next dev` server caches this route aggressively and can serve a stale
 * `https://tokscale.ai/`; check prod, not dev, if the two ever look different.)
 */
export function homeUrl(): string {
  return SITE_URL;
}

/**
 * The leaderboard accepts period, sortBy, page, from/to, search and view.
 *
 * All of them except `view` are filters over one ranking, so they collapse onto
 * the bare URL — `search` especially, since it spans an unbounded set of URLs
 * that would otherwise be crawled as distinct near-duplicate pages.
 *
 * `view=groups` is the exception: it renders the groups browser, which is a
 * different page that /groups permanently redirects to, so it canonicalizes to
 * itself rather than collapsing into the user leaderboard.
 */
export function leaderboardUrl(view: "users" | "groups" = "users"): string {
  return view === "groups"
    ? `${SITE_URL}/leaderboard?view=groups`
    : `${SITE_URL}/leaderboard`;
}

/**
 * Profiles accept ?period=all|week|month. All three render the same profile
 * over a different window, so the bare URL is canonical.
 *
 * `username` must be the DB's casing: /u/[username] permanently redirects any
 * other casing, so a canonical built from the raw request param could point at
 * a redirect.
 */
export function profileUrl(username: string): string {
  return `${SITE_URL}/u/${encodeURIComponent(username)}`;
}

export function groupUrl(slug: string): string {
  return `${SITE_URL}/groups/${encodeURIComponent(slug)}`;
}

/**
 * Static informational pages. Listed here rather than inlined so they land in
 * the sitemap and their canonical tags through the same path as everything
 * else — ad networks and search engines both check that these exist and are
 * reachable, so they must not be the one set of URLs that drifts.
 */
export const LEGAL_PATHS = ["privacy", "terms", "contact"] as const;

export type LegalPath = (typeof LEGAL_PATHS)[number];

export function legalUrl(page: LegalPath): string {
  return `${SITE_URL}/${page}`;
}
