import type { Metadata } from "next";
import Link from "next/link";
import { notFound } from "next/navigation";
import { TOKSCALE_X_HANDLE } from "@/lib/community";
import { absoluteUrl, capitalPath, investorsPath, investorUrl } from "@/lib/capital";
import { formatPartialDate, ledger } from "@/lib/capital/ledger";
import { Amount, Missing } from "../../_components/Amount";
import { Breadcrumbs } from "../../_components/Breadcrumbs";
import { CapitalShell } from "../../_components/CapitalShell";
import { EventFeed } from "../../_components/EventFeed";
import { JsonLd } from "../../_components/JsonLd";
import { OrgMark } from "../../_components/OrgMark";
import { SourceLink, sourceShortLabel } from "../../_components/SourceLink";
import styles from "../../_components/capital.module.css";
import { titleCase } from "@/lib/text/titleCase";

interface PageProps {
  params: Promise<{ slug: string }>;
}

export const dynamicParams = false;

export function generateStaticParams() {
  return ledger.getInvestors().map((row) => ({ slug: row.org.slug }));
}

export async function generateMetadata({ params }: PageProps): Promise<Metadata> {
  const { slug } = await params;
  const p = ledger.getInvestor(slug);
  if (!p) return { title: "Not found | Tokscale", robots: { index: false } };
  const title = `${p.org.name}: AI Investments Tokscale Tracks | Tokscale`;
  const description = `${p.org.name} in ${p.participations.length} AI ${p.participations.length === 1 ? "round or deal" : "rounds and deals"} Tokscale tracks, with roles and primary sources.`;
  const canonical = investorUrl(slug);
  return {
    title,
    description,
    alternates: { canonical },
    robots: p.indexable ? undefined : { index: false, follow: true },
    openGraph: { title, description, type: "profile", url: canonical, siteName: "Tokscale" },
    twitter: { card: "summary", site: TOKSCALE_X_HANDLE, title, description },
  };
}

export default async function InvestorPage({ params }: PageProps) {
  const { slug } = await params;
  const p = ledger.getInvestor(slug);
  if (!p) notFound();
  const org = p.org.org;
  const canonical = investorUrl(slug);

  return (
    <CapitalShell section="investors">
      <JsonLd
        data={{
          "@context": "https://schema.org",
          "@graph": [
            {
              "@type": "Organization",
              name: org.name,
              ...(org.legalNames[0] ? { legalName: org.legalNames[0] } : {}),
              ...(org.website ? { url: org.website } : {}),
              ...(org.ids.wikidata || org.website
                ? { sameAs: [org.website, org.ids.wikidata ? `https://www.wikidata.org/wiki/${org.ids.wikidata}` : null].filter(Boolean) }
                : {}),
              ...(org.ids.secAdviser ? { identifier: { "@type": "PropertyValue", propertyID: "SEC adviser number", value: org.ids.secAdviser } } : {}),
              description: org.description,
              mainEntityOfPage: canonical,
            },
            {
              "@type": "BreadcrumbList",
              itemListElement: [
                { "@type": "ListItem", position: 1, name: "Capital", item: absoluteUrl(capitalPath()) },
                { "@type": "ListItem", position: 2, name: "Investors", item: absoluteUrl(investorsPath()) },
                { "@type": "ListItem", position: 3, name: org.name, item: canonical },
              ],
            },
          ],
        }}
      />
      <Breadcrumbs items={[{ label: "Capital", href: capitalPath() }, { label: "Investors", href: investorsPath() }, { label: org.name }]} />
      <header className={styles.profileHead}>
        <OrgMark org={p.org} large />
        <div>
          <h1 className={`${styles.display} ${styles.profileTitle}`}>{org.name}</h1>
          <p className={styles.lede}>
            {titleCase(
              [p.org.investorTypeLabels.join(", "), org.hq ? [org.hq.city, org.hq.country].filter(Boolean).join(", ") : null]
                .filter(Boolean)
                .join(" · "),
            )}
            . {org.description}
          </p>
          <ul className={styles.idLinks}>
            {org.website ? (
              <li>
                <a href={org.website} target="_blank" rel="noopener noreferrer">
                  {new URL(org.website).hostname.replace(/^www\./, "")}
                </a>
              </li>
            ) : null}
            {org.ids.wikidata ? (
              <li>
                <a href={`https://www.wikidata.org/wiki/${org.ids.wikidata}`} target="_blank" rel="noopener noreferrer">
                  Wikidata {org.ids.wikidata}
                </a>
              </li>
            ) : null}
            {p.companyHref ? (
              <li>
                <Link href={p.companyHref}>{org.name} company profile</Link>
              </li>
            ) : null}
          </ul>
        </div>
      </header>

      {p.participations.length > 0 ? (
        <>
          <dl className={styles.stats} style={{ "--cols": 4 } as React.CSSProperties} aria-label={`${org.name} in tracked rounds`}>
            <div className={styles.stat}>
              <dt className={styles.statLabel}>Rounds and deals tracked</dt>
              <dd className={styles.statValue}>{p.participations.length}</dd>
            </div>
            <div className={styles.stat}>
              <dt className={styles.statLabel}>Led or co-led</dt>
              <dd className={styles.statValue}>{p.leadCount}</dd>
            </div>
            <div className={styles.stat}>
              <dt className={styles.statLabel}>First tracked</dt>
              <dd className={styles.statValue}>{p.firstDate ? formatPartialDate(p.firstDate) : "—"}</dd>
            </div>
            <div className={styles.stat}>
              <dt className={styles.statLabel}>Latest tracked</dt>
              <dd className={styles.statValue}>{p.latestDate ? formatPartialDate(p.latestDate) : "—"}</dd>
            </div>
          </dl>

          <section className={styles.section} style={{ marginTop: 32 }} aria-labelledby="rounds">
            <div className={styles.sectionHead}>
              <h2 id="rounds" className={`${styles.display} ${styles.sectionTitle}`}>
                AI rounds and deals
              </h2>
              <p className={styles.sectionNote}>Roles as named in the primary source</p>
            </div>
            <table className={`${styles.ledgerTable} ${styles.ledgerStack}`}>
              <thead>
                <tr>
                  <th scope="col">Date</th>
                  <th scope="col">Event</th>
                  <th scope="col">Role</th>
                  <th scope="col" className={styles.right}>
                    Size
                  </th>
                  <th scope="col" className={styles.right}>
                    Post-money
                  </th>
                  <th scope="col">Source</th>
                </tr>
              </thead>
              <tbody>
                {p.participations.map(({ event, roleLabels }) => {
                  const size = event.amounts.find((a) => a.skipped === null && ["equity_raised", "debt_raised", "ipo_proceeds", "equity_investment", "purchase_price"].includes(a.amount.measure));
                  return (
                    <tr key={event.id} data-row-link="">
                      <td className={styles.nowrap} data-label="Date">
                        {formatPartialDate(event.date)}
                      </td>
                      <td className={styles.full}>
                        <Link className={styles.cellMain} href={event.href} data-row-primary="">
                          {event.title}
                        </Link>
                        <span className={styles.cellSub}>{event.issuer ? event.issuer.name : event.partiesText}</span>
                      </td>
                      <td data-label="Role">{roleLabels.join(", ")}</td>
                      <td className={styles.right} data-label="Size">
                        {size ? <Amount view={size} /> : <Missing reason="No amount stated" />}
                      </td>
                      <td className={styles.right} data-label="Post-money">
                        {event.valuation ? <Amount view={event.valuation} /> : <Missing />}
                      </td>
                      <td data-label="Source">
                        <SourceLink source={size?.source ?? event.valuation?.source ?? event.sources[0]} />
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
            <p className={styles.note}>
              Size is the whole round or deal, not this investor’s share. Amounts per investor appear only when a source states
              them
              {p.ownAmounts.length > 0
                ? `: ${p.ownAmounts.map((a) => `${a.short} (${a.amount.label ?? a.amount.measure}, ${formatPartialDate(a.amount.asOf)}${a.skipped === "included_in" ? ", counted in a later round" : ""})`).join("; ")}`
                : ""}
              .
            </p>
          </section>

          {p.portfolio.length > 0 ? (
            <section className={styles.section} aria-labelledby="portfolio">
              <div className={styles.sectionHead}>
                <h2 id="portfolio" className={`${styles.display} ${styles.sectionTitle}`}>
                  Companies in tracked rounds
                </h2>
              </div>
              <ul className={styles.chips}>
                {p.portfolio.map((o) => (
                  <li key={o.id}>{o.companyHref ? <Link href={o.companyHref}>{o.name}</Link> : o.name}</li>
                ))}
              </ul>
            </section>
          ) : null}
        </>
      ) : (
        <p className={styles.callout}>
          Tokscale has no round or deal with a primary source that names {org.name} yet. This page stays out of search results
          until it has one.
        </p>
      )}

      {p.otherEvents.length > 0 ? (
        <section className={styles.section} aria-labelledby="other">
          <div className={styles.sectionHead}>
            <h2 id="other" className={`${styles.display} ${styles.sectionTitle}`}>
              Other deals
            </h2>
            <p className={styles.sectionNote}>Compute, leases and acquisitions with {org.name} as a party</p>
          </div>
          <EventFeed events={p.otherEvents} />
        </section>
      ) : null}

      {p.sources.length > 0 ? (
        <section className={styles.section} aria-labelledby="sources">
          <div className={styles.sectionHead}>
            <h2 id="sources" className={`${styles.display} ${styles.sectionTitle}`}>
              Sources
            </h2>
            <p className={styles.sectionNote}>Reviewed {formatPartialDate(p.reviewedAt)}</p>
          </div>
          <ol className={styles.sourcesOl}>
            {p.sources.map((s) => (
              <li key={s.id}>
                {s.publisher}, “
                <a href={s.url} target="_blank" rel="noopener noreferrer">
                  {s.title}
                </a>
                ”{s.publishedAt ? `, ${formatPartialDate(s.publishedAt)}` : ""} · {sourceShortLabel(s)}
              </li>
            ))}
          </ol>
        </section>
      ) : null}
    </CapitalShell>
  );
}
