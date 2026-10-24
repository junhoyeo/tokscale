import { getPublicOrigin } from "@/lib/seo/urls";

/** Path builders for the capital section. Absolute variants share the site's canonical origin. */
export const capitalPath = () => "/capital";
export const companiesPath = () => "/capital/companies";
/** The directory as CSV (route handler; not indexed). */
export const companiesCsvPath = () => "/capital/companies.csv";
/** Company pages live inside Capital; model pages link their lab here too. */
export const companyPath = (slug: string) => `/capital/companies/${encodeURIComponent(slug)}`;
export const investorsPath = (type?: string) =>
  type ? `/capital/investors?type=${encodeURIComponent(type)}` : "/capital/investors";
export const investorPath = (slug: string) => `/capital/investors/${encodeURIComponent(slug)}`;
export const roundsPath = () => "/capital/rounds";
export const roundPath = (slug: string) => `/capital/rounds/${encodeURIComponent(slug)}`;
export const dealPath = (slug: string) => `/capital/deals/${encodeURIComponent(slug)}`;
export const computePath = () => "/capital/compute";
export const spendingPath = () => "/capital/spending";
export const sectorPath = (slug: string) => `/capital/sectors/${encodeURIComponent(slug)}`;
export const methodologyPath = () => "/capital/methodology";

/** 1200x630 Open Graph cards (route handlers next to the pages). */
export const roundOgImagePath = (slug: string) => `${roundPath(slug)}/og`;
export const dealOgImagePath = (slug: string) => `${dealPath(slug)}/og`;

export const absoluteUrl = (path: string) => `${getPublicOrigin()}${path}`;
export const capitalUrl = () => absoluteUrl(capitalPath());
export const companyUrl = (slug: string) => absoluteUrl(companyPath(slug));
export const companiesUrl = () => absoluteUrl(companiesPath());
export const investorsUrl = () => absoluteUrl(investorsPath());
export const investorUrl = (slug: string) => absoluteUrl(investorPath(slug));
export const roundsUrl = () => absoluteUrl(roundsPath());
export const roundUrl = (slug: string) => absoluteUrl(roundPath(slug));
export const dealUrl = (slug: string) => absoluteUrl(dealPath(slug));
export const computeUrl = () => absoluteUrl(computePath());
export const spendingUrl = () => absoluteUrl(spendingPath());
export const sectorUrl = (slug: string) => absoluteUrl(sectorPath(slug));
export const methodologyUrl = () => absoluteUrl(methodologyPath());

/** SEC EDGAR filing list for a CIK (all forms). */
export const edgarFilingsUrl = (cik: string) =>
  `https://www.sec.gov/cgi-bin/browse-edgar?action=getcompany&CIK=${cik}&owner=include&count=40`;
