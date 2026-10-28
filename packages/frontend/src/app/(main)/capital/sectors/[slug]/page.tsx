import type { Metadata } from "next";
import Link from "next/link";
import { notFound } from "next/navigation";
import { TOKSCALE_X_HANDLE } from "@/lib/community";
import { absoluteUrl, capitalPath, companiesPath, sectorPath, sectorUrl, spendingPath } from "@/lib/capital";
import { SECTORS, financingLabel, formatMoney, formatPartialDate, ledger } from "@/lib/capital/ledger";
import { calendarSpanLabel, fiscalLabel } from "@/lib/capital/ledger/capex";
import { Amount, Missing } from "../../_components/Amount";
import { Breadcrumbs } from "../../_components/Breadcrumbs";
import { CapexBars } from "../../_components/CapexBars";
import { CapitalShell } from "../../_components/CapitalShell";
import { EventFeed } from "../../_components/EventFeed";
import { JsonLd } from "../../_components/JsonLd";
import { OrgCell } from "../../_components/OrgMark";
import styles from "../../_components/capital.module.css";

interface PageProps {
  params: Promise<{ slug: string }>;
}

export const dynamicParams = false;

export function generateStaticParams() {
  return SECTORS.map((s) => ({ slug: s.slug }));
}

const SEO: Record<string, string> = {
  "ai-labs": "AI Labs: Funding, Valuations and Compute | Tokscale",
  neoclouds: "Neoclouds: GPU Cloud Contracts, Capex and Financing | Tokscale",
  hyperscalers: "Hyperscalers: AI Capex and Compute Deals | Tokscale",
  semiconductors: "AI Chip Companies: Deals, Capex and Investments | Tokscale",
  "data-centers": "AI Data-Center Developers: Leases and Financing | Tokscale",
  power: "Power for AI: Utilities, Nuclear and Data-Center Deals | Tokscale",
};

export async function generateMetadata({ params }: PageProps): Promise<Metadata> {
  const { slug } = await params;
  const sector = ledger.getSector(slug);
  if (!sector) return { title: "Not found | Tokscale", robots: { index: false } };
  const title = SEO[slug] ?? `${sector.name} | Tokscale`;
  const description = `${sector.summary} ${sector.companies.length} companies and ${sector.events.length} tracked events, each with its source.`;
  return {
    title,
    description,
    alternates: { canonical: sectorUrl(slug) },
    openGraph: { title, description, type: "website", url: sectorUrl(slug), siteName: "Tokscale" },
    twitter: { card: "summary", site: TOKSCALE_X_HANDLE, title, description },
  };
}

export default async function SectorPage({ params }: PageProps) {
  const { slug } = await params;
  const sector = ledger.getSector(slug);
  if (!sector) notFound();
  const rows = ledger.getDirectoryRows().filter((r) => sector.companies.some((c) => c.id === r.org.id));
  rows.sort((a, b) => (b.lastActivity ?? "").localeCompare(a.lastActivity ?? "") || a.org.name.localeCompare(b.org.name));

  return (
    <CapitalShell section="companies">
      <JsonLd
        data={{
          "@context": "https://schema.org",
          "@type": "ItemList",
          name: sector.name,
          url: sectorUrl(slug),
          numberOfItems: rows.length,
          itemListElement: rows.map((r, i) => ({ "@type": "ListItem", position: i + 1, name: r.org.name, url: absoluteUrl(r.org.companyHref ?? companiesPath()) })),
        }}
      />
      <Breadcrumbs items={[{ label: "Capital", href: capitalPath() }, { label: "Companies", href: companiesPath() }, { label: sector.name }]} />
      <header className={styles.pageHead}>
        <h1 className={`${styles.display} ${styles.pageTitle}`}>{sector.name}</h1>
        <p className={styles.lede}>{sector.summary}</p>
        <p className={styles.metaLine}>
          <span>{rows.length} companies</span>
          <span>{sector.events.length} events</span>
          {SECTORS.filter((s) => s.slug !== slug).map((s) => (
            <span key={s.slug}>
              <Link className={styles.link} href={sectorPath(s.slug)}>
                {s.name}
              </Link>
            </span>
          ))}
        </p>
      </header>

      <table className={`${styles.ledgerTable} ${styles.ledgerStack}`}>
        <thead>
          <tr>
            <th scope="col">Company</th>
            <th scope="col">Status</th>
            <th scope="col">Last financing</th>
            <th scope="col" className={styles.right}>
              Post-money
            </th>
            <th scope="col" className={styles.right}>
              Compute deals
            </th>
            <th scope="col" className={styles.right}>
              Latest quarter capex
            </th>
          </tr>
        </thead>
        <tbody>
          {rows.map((row) => (
            <tr key={row.org.id} data-row-link={row.org.href ? "" : undefined}>
              <td className={styles.full}>
                <OrgCell org={row.org} rowLink />
              </td>
              <td data-label="Status">{row.org.statusText}</td>
              <td className={row.lastFinancing ? undefined : styles.emptyStacked} data-label="Last financing">
                {row.lastFinancing ? (
                  <>
                    <Link href={row.lastFinancing.href}>{financingLabel(row.lastFinancing)}</Link>
                    <span className={styles.cellSub}>{formatPartialDate(row.lastFinancing.date)}</span>
                  </>
                ) : (
                  <Missing />
                )}
              </td>
              <td className={`${styles.right} ${row.valuation ? "" : styles.emptyStacked}`} data-label="Post-money">
                {row.valuation ? <Amount view={row.valuation} /> : <Missing />}
              </td>
              <td className={`${styles.right} ${styles.num} ${row.dealsCount ? "" : styles.emptyStacked}`} data-label="Compute deals">
                {row.dealsCount || <Missing />}
              </td>
              <td className={`${styles.right} ${row.capexLatest ? "" : styles.emptyStacked}`} data-label="Latest quarter capex">
                {row.capexLatest ? (
                  <>
                    <span className={styles.num}>{formatMoney(row.capexLatest.value, row.capexLatest.currency)}</span>
                    <span className={styles.cellSub}>
                      {fiscalLabel(row.capexLatest)} · {calendarSpanLabel(row.capexLatest.start, row.capexLatest.end)}
                      {row.capexLatest.derived ? " · derived" : ""}
                    </span>
                  </>
                ) : (
                  <Missing />
                )}
              </td>
            </tr>
          ))}
        </tbody>
      </table>

      {sector.capex.length > 0 ? (
        <section className={styles.section} style={{ marginTop: 40 }} aria-labelledby="capex">
          <div className={styles.sectionHead}>
            <h2 id="capex" className={`${styles.display} ${styles.sectionTitle}`}>
              Cash paid for property and equipment
            </h2>
            <p className={styles.sectionNote}>
              <Link className={styles.link} href={spendingPath()}>
                All companies →
              </Link>
            </p>
          </div>
          <ul className={styles.multiples}>
            {sector.capex
              .filter((v) => v.quarters.length > 0)
              .map((view) => {
                const org = ledger.orgView(view.org);
                const last = view.quarters.at(-1)!;
                return (
                  <li className={styles.multiple} key={view.org}>
                    <p className={styles.multipleHead}>
                      {org.companyHref ? <Link href={org.companyHref}>{org.displayName}</Link> : <span>{org.displayName}</span>}
                      <span className={styles.num}>{formatMoney(last.value, last.currency)}</span>
                    </p>
                    <CapexBars quarters={view.quarters} label={`${org.name} quarterly cash capex`} />
                  </li>
                );
              })}
          </ul>
        </section>
      ) : null}

      <section className={styles.section} style={sector.capex.length > 0 ? undefined : { marginTop: 40 }} aria-labelledby="events">
        <div className={styles.sectionHead}>
          <h2 id="events" className={`${styles.display} ${styles.sectionTitle}`}>
            Events
          </h2>
        </div>
        <EventFeed events={sector.events} />
      </section>
    </CapitalShell>
  );
}
