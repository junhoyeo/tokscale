import type { Metadata } from "next";
import Link from "next/link";
import { TOKSCALE_X_HANDLE } from "@/lib/community";
import { capitalPath, investorsPath, investorsUrl } from "@/lib/capital";
import { INVESTOR_TYPES, INVESTOR_TYPE_LABELS, formatPartialDate, ledger, type InvestorType } from "@/lib/capital/ledger";
import { Breadcrumbs } from "../_components/Breadcrumbs";
import { CapitalShell } from "../_components/CapitalShell";
import { JsonLd } from "../_components/JsonLd";
import { OrgCell } from "../_components/OrgMark";
import { Missing } from "../_components/Amount";
import styles from "../_components/capital.module.css";

const TITLE = "AI Investors: Rounds, Leads and Deals | Tokscale";
const DESCRIPTION =
  "Who funds the AI economy: venture, growth, sovereign and corporate investors in the rounds and deals Tokscale tracks, with their roles and primary sources.";

export const metadata: Metadata = {
  title: TITLE,
  description: DESCRIPTION,
  alternates: { canonical: investorsUrl() },
  openGraph: { title: TITLE, description: DESCRIPTION, type: "website", url: investorsUrl(), siteName: "Tokscale" },
  twitter: { card: "summary_large_image", site: TOKSCALE_X_HANDLE, title: TITLE, description: DESCRIPTION },
};

/** URL keys for the type filter. */
const TYPE_KEYS: Record<string, InvestorType> = {
  vc: "vc",
  growth: "growth",
  sovereign: "sovereign",
  corporate: "corporate",
  "asset-manager": "asset-manager",
  "registered-fund": "registered-fund",
};
const keyOf = (type: InvestorType) => Object.entries(TYPE_KEYS).find(([, t]) => t === type)![0];

interface PageProps {
  searchParams: Promise<{ type?: string | string[] }>;
}

export default async function InvestorsPage({ searchParams }: PageProps) {
  const raw = (await searchParams).type;
  const typeKey = (Array.isArray(raw) ? raw[0] : raw) ?? "";
  const type = TYPE_KEYS[typeKey] ?? null;
  const all = ledger.getInvestors();
  const rows = all
    .filter((row) => !type || row.org.org.investorTypes.includes(type))
    .sort((a, b) => b.rounds - a.rounds || (b.latest?.date ?? "").localeCompare(a.latest?.date ?? "") || a.org.name.localeCompare(b.org.name));
  const counts = new Map(INVESTOR_TYPES.map((t) => [t, all.filter((r) => r.org.org.investorTypes.includes(t)).length] as const));

  return (
    <CapitalShell section="investors">
      <JsonLd
        data={{
          "@context": "https://schema.org",
          "@type": "ItemList",
          name: "AI investors Tokscale tracks",
          url: investorsUrl(),
          numberOfItems: all.length,
          itemListElement: all.map((row, index) => ({
            "@type": "ListItem",
            position: index + 1,
            name: row.org.name,
            url: investorsUrl().replace(/\/capital\/investors$/, row.org.investorHref ?? ""),
          })),
        }}
      />
      <Breadcrumbs items={[{ label: "Capital", href: capitalPath() }, { label: "Investors" }]} />
      <header className={styles.pageHead}>
        <h1 className={`${styles.display} ${styles.pageTitle}`}>Who funds the AI economy</h1>
        <p className={styles.lede}>Investors named in the primary sources of the rounds and deals Tokscale tracks.</p>
      </header>

      <nav aria-label="Investor type">
        <ul className={styles.views}>
          <li>
            <Link href={investorsPath()} aria-current={!type ? "true" : undefined} scroll={false}>
              All {all.length}
            </Link>
          </li>
          {INVESTOR_TYPES.filter((t) => (counts.get(t) ?? 0) > 0).map((t) => (
            <li key={t}>
              <Link href={investorsPath(keyOf(t))} aria-current={type === t ? "true" : undefined} scroll={false}>
                {INVESTOR_TYPE_LABELS[t]} {counts.get(t)}
              </Link>
            </li>
          ))}
        </ul>
      </nav>

      <section aria-label="Investors">
        <table className={`${styles.ledgerTable} ${styles.ledgerStack}`}>
          <thead>
            <tr>
              <th scope="col">Investor</th>
              <th scope="col">Type</th>
              <th scope="col" className={styles.right}>
                Rounds and deals
              </th>
              <th scope="col" className={styles.right}>
                Led or co-led
              </th>
              <th scope="col">Latest</th>
              <th scope="col">Companies</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((row) => (
              <tr key={row.org.id} data-row-link={row.org.investorHref ? "" : undefined}>
                <td className={styles.full}>
                  <OrgCell org={{ ...row.org, href: row.org.investorHref }} rowLink />
                </td>
                <td data-label="Type">{row.org.investorTypeLabels.join(", ")}</td>
                <td className={`${styles.right} ${styles.num}`} data-label="Rounds and deals">
                  {row.rounds > 0 ? row.rounds : <Missing reason="No tracked round yet" />}
                </td>
                <td className={`${styles.right} ${styles.num}`} data-label="Led or co-led">
                  {row.leads > 0 ? row.leads : <Missing reason="Not a lead in a tracked round" />}
                </td>
                <td data-label="Latest">
                  {row.latest ? (
                    <>
                      <Link href={row.latest.href}>{row.latest.title}</Link>
                      <span className={styles.cellSub}>{formatPartialDate(row.latest.date)}</span>
                    </>
                  ) : (
                    <Missing />
                  )}
                </td>
                <td data-label="Companies">{row.portfolio.length > 0 ? row.portfolio.join(", ") : <Missing />}</td>
              </tr>
            ))}
          </tbody>
        </table>
        {rows.length === 0 ? <p className={styles.empty}>No investor of this type yet.</p> : null}
        <p className={styles.note}>
          Counts are rounds and deals in Tokscale’s ledger, not an investor’s full record. Investors without a tracked round
          have a page but stay out of search results until they have a sourced fact.
        </p>
      </section>
    </CapitalShell>
  );
}
