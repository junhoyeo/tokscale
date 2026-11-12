/**
 * Whether this deployment builds public pages ahead of time and refreshes them
 * in the background, so visitors get them from the CDN.
 *
 * A Vercel build already knows the site origin. A self-hosted image learns
 * APP_URL only when its container starts, so there every page is rendered per
 * request (see generateMetadata in app/layout.tsx).
 *
 * A page still opts in with its own `revalidate`, and any page that reads
 * cookies, headers or search params stays per-request on both.
 */
export function isPrerenderedDeployment(): boolean {
  return Boolean(process.env.VERCEL);
}
