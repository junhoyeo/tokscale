/**
 * Capital URLs that no longer have a page, and where each one goes now. next.config.ts serves
 * them as permanent (308) redirects, so shared and indexed links do not end on a 404.
 *
 * Tokscale no longer publishes 13F holdings. These URLs showed them:
 * - /capital/portfolios and /capital/funds/**: older 13F URLs. They go to /capital/investors.
 * - /capital/investors/:slug/13f/**: 13F quarter pages and their Open Graph cards. They go to
 *   /capital/investors.
 * - /capital/investors/:slug for REMOVED_INVESTOR_SLUGS: the 13F manager profiles. They go to
 *   /capital/investors.
 * - /capital/companies/:slug for REMOVED_COMPANY_SLUGS: listed companies whose page showed only
 *   13F positions. They go to /capital/companies.
 *
 * A redirect runs before a page with the same path. So a slug in these lists must not come back
 * as a company or an investor in the ledger. The capital tests check this.
 */
const INVESTORS = "/capital/investors";
const COMPANIES = "/capital/companies";

export const REMOVED_INVESTOR_SLUGS = ["pershing-square", "thiel-macro"] as const;

export const REMOVED_COMPANY_SLUGS = [
  "american-electric-power",
  "apple",
  "brookfield",
  "chipotle",
  "cms-energy",
  "dte-energy",
  "firstenergy",
  "hertz",
  "hilton",
  "howard-hughes",
  "mastercard",
  "netflix",
  "pershing-square-usa",
  "restaurant-brands-international",
  "seaport-entertainment",
  "sp-global",
  "tesla",
  "uber",
  "visa",
  "vista-energy",
] as const;

export interface CapitalRedirect {
  source: string;
  destination: string;
  permanent: true;
}

export const CAPITAL_REDIRECTS: readonly CapitalRedirect[] = [
  { source: "/capital/portfolios", destination: INVESTORS, permanent: true },
  { source: "/capital/funds/:path*", destination: INVESTORS, permanent: true },
  { source: "/capital/investors/:slug/13f/:path*", destination: INVESTORS, permanent: true },
  ...REMOVED_INVESTOR_SLUGS.map((slug) => ({ source: `${INVESTORS}/${slug}`, destination: INVESTORS, permanent: true as const })),
  ...REMOVED_COMPANY_SLUGS.map((slug) => ({ source: `${COMPANIES}/${slug}`, destination: COMPANIES, permanent: true as const })),
];
