import { ledger } from "@/lib/capital/ledger";

/**
 * Explore labels a model's lab with a lab slug ("google"); Capital company pages are keyed
 * by the company ("alphabet"). The mapping is the `labSlugs` field in
 * src/content/capital/orgs.json, so it lives next to the company data and the content
 * validation rejects a lab slug that points at two companies.
 */
const LAB_TO_COMPANY: ReadonlyMap<string, string> = new Map(
  ledger
    .getCompanies()
    .flatMap((company) => company.org.labSlugs.map((lab) => [lab, company.slug] as const))
);

/** Company slug to link a lab to, or null when no company page exists. */
export function companySlugForLab(labSlug: string | null): string | null {
  if (!labSlug) return null;
  return LAB_TO_COMPANY.get(labSlug) ?? null;
}
