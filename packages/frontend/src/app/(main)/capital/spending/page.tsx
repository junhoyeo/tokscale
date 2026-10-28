import type { Metadata } from "next";
import Link from "next/link";
import { TOKSCALE_X_HANDLE } from "@/lib/community";
import { capitalPath, methodologyPath, spendingUrl } from "@/lib/capital";
import { formatMoney, formatPartialDate, formatTotal, ledger } from "@/lib/capital/ledger";
import { calendarSpanLabel, fiscalLabel } from "@/lib/capital/ledger/capex";
import { AmountBar, Missing, maxByCurrency } from "../_components/Amount";
import { Breadcrumbs } from "../_components/Breadcrumbs";
import { CapexBars } from "../_components/CapexBars";
import { CapexSources } from "../_components/CapexSources";
import { CapitalShell } from "../_components/CapitalShell";
import { JsonLd } from "../_components/JsonLd";
import { OrgMark } from "../_components/OrgMark";
import styles from "../_components/capital.module.css";

const TITLE = "AI Capex Tracker: Cash Spending by Microsoft, Amazon, Alphabet, Meta and Oracle | Tokscale";
const DESCRIPTION =
  "Cash paid for property and equipment by quarter for hyperscalers, neoclouds, chip makers and power companies, from SEC XBRL filings, with fiscal labels and derived quarters flagged.";

export const metadata: Metadata = {
  title: TITLE,
  description: DESCRIPTION,
  alternates: { canonical: spendingUrl() },
  openGraph: { title: TITLE, description: DESCRIPTION, type: "website", url: spendingUrl(), siteName: "Tokscale" },
  twitter: { card: "summary_large_image", site: TOKSCALE_X_HANDLE, title: TITLE, description: DESCRIPTION },
};

export default function SpendingPage() {
  const rows = ledger.getCapex();
  const quarter = ledger.capexCalendarQuarter();
  const retrieved = ledger.capexRetrievedAt.slice(0, 10);
  const years = rows.flatMap((r) => r.view.quarters.map((q) => q.start.slice(0, 4))).sort();
  // Bars are scaled inside one currency; nothing on this page is converted.
  const scale = maxByCurrency(
    rows.map(({ view }) => {
      const last = view.quarters.at(-1) ?? null;
      return { value: last?.value ?? null, currency: last?.currency ?? "" };
    })
  );
  const inSum = new Set(quarter ? quarter.entries.map((e) => e.org.id) : []);
  const leftOut = quarter ? rows.filter((r) => !inSum.has(r.org.id)) : [];

  return (
    <CapitalShell section="spending">
      <JsonLd
        data={{
          "@context": "https://schema.org",
          "@type": "Dataset",
          name: "Cash paid for property and equipment by quarter",
          description: DESCRIPTION,
          url: spendingUrl(),
          creator: { "@type": "Organization", name: "Tokscale" },
          isBasedOn: "https://www.sec.gov/search-filings/edgar-application-programming-interfaces",
          temporalCoverage: years.length > 0 ? `${years[0]}/${retrieved.slice(0, 4)}` : undefined,
          dateModified: retrieved,
          variableMeasured: "Cash paid for property and equipment (purchases of property, plant and equipment)",
        }}
      />
      <Breadcrumbs items={[{ label: "Capital", href: capitalPath() }, { label: "Spending" }]} />
      <header className={styles.pageHead}>
        <h1 className={`${styles.display} ${styles.pageTitle}`}>Cash paid for property and equipment</h1>
        <p className={styles.lede}>
          How much the companies that build AI capacity spend on servers, buildings and power equipment, quarter by quarter,
          from their SEC filings. This is cash paid, not capex guidance, and it leaves out finance leases.
        </p>
        <p className={styles.metaLine}>
          <span>{rows.length} companies</span>
          <span>SEC XBRL facts retrieved {formatPartialDate(retrieved)}</span>
          <span>Fiscal labels kept</span>
        </p>
      </header>

      {quarter ? (
        <section aria-labelledby="calendar-quarter">
          <dl className={styles.stats} style={{ "--cols": 3 } as React.CSSProperties}>
            <div className={styles.stat}>
              <dt className={styles.statLabel} id="calendar-quarter">
                {quarter.label}, {quarter.entries.length} companies
              </dt>
              <dd className={styles.statValue}>
                <span className={styles.num}>{formatTotal(quarter.total)}</span>
              </dd>
              <dd className={styles.statSub}>
                {calendarSpanLabel(quarter.entries[0].quarter.start, quarter.entries[0].quarter.end)}, only companies whose fiscal
                quarter is this calendar quarter
              </dd>
            </div>
            <div className={styles.stat}>
              <dt className={styles.statLabel}>Derived quarters in this sum</dt>
              <dd className={styles.statValue}>{quarter.derivedCount}</dd>
              <dd className={styles.statSub}>Year to date minus the earlier year to date, computed by Tokscale</dd>
            </div>
            <div className={styles.stat}>
              <dt className={styles.statLabel}>Largest</dt>
              <dd className={styles.statValue}>{quarter.entries[0].org.displayName}</dd>
              <dd className={styles.statSub}>{formatMoney(quarter.entries[0].quarter.value, "USD")}</dd>
            </div>
          </dl>
          {leftOut.length > 0 ? (
            <p className={styles.note}>
              Left out of this sum because they have no figure for exactly this calendar quarter in USD (a different fiscal
              calendar, another currency or no quarterly filing): {leftOut.map((r) => r.org.displayName).join(", ")}. They are
              listed below with their own fiscal labels.
            </p>
          ) : null}
        </section>
      ) : null}

      <section className={styles.section} style={{ marginTop: 32 }} aria-labelledby="multiples">
        <div className={styles.sectionHead}>
          <h2 id="multiples" className={`${styles.display} ${styles.sectionTitle}`}>
            Last eight quarters
          </h2>
          <p className={styles.sectionNote}>
            <span className={styles.legendKey}>
              <span className={styles.keySolid} aria-hidden="true" /> Reported
            </span>
            <span className={styles.legendKey}>
              <span className={styles.keyHollow} aria-hidden="true" /> Derived
            </span>
          </p>
        </div>
        <ul className={styles.multiples}>
          {rows
            .filter((r) => r.view.quarters.length > 0)
            .map(({ org, view }) => {
              const last = view.quarters.at(-1)!;
              return (
                <li className={styles.multiple} key={org.id}>
                  <p className={styles.multipleHead}>
                    {org.companyHref ? <Link href={`${org.companyHref}#spending`}>{org.displayName}</Link> : <span>{org.displayName}</span>}
                    <span className={styles.num}>{formatMoney(last.value, last.currency)}</span>
                  </p>
                  <CapexBars quarters={view.quarters} label={`${org.name} quarterly cash capex`} />
                  <p className={styles.multipleFoot}>
                    <span>{fiscalLabel(last)}</span>
                    <span>{calendarSpanLabel(last.start, last.end)}</span>
                  </p>
                </li>
              );
            })}
        </ul>
      </section>

      <section className={styles.section} aria-labelledby="table">
        <div className={styles.sectionHead}>
          <h2 id="table" className={`${styles.display} ${styles.sectionTitle}`}>
            All companies
          </h2>
          <p className={styles.sectionNote}>Original currency; nothing is converted</p>
        </div>
        <table className={`${styles.ledgerTable} ${styles.ledgerStack}`}>
          <thead>
            <tr>
              <th scope="col">Company</th>
              <th scope="col">Latest quarter</th>
              <th scope="col" className={`${styles.right} ${styles.amountCell}`}>
                Cash paid
              </th>
              <th scope="col" className={styles.right}>
                Last 4 quarters
              </th>
              <th scope="col" className={styles.right}>
                Latest fiscal year
              </th>
              <th scope="col">Line item and notes</th>
            </tr>
          </thead>
          <tbody>
            {rows.map(({ org, view }) => {
              const last = view.quarters.at(-1) ?? null;
              const lastFour = view.quarters.slice(-4);
              const contiguous =
                lastFour.length === 4 &&
                lastFour.every((q, i) => i === 0 || Date.parse(`${q.start}T00:00:00Z`) - Date.parse(`${lastFour[i - 1].end}T00:00:00Z`) <= 2 * 86_400_000);
              const year = view.years.at(-1) ?? null;
              return (
                <tr key={org.id} data-row-link={org.companyHref ? "" : undefined}>
                  <td className={`${styles.full} ${styles.nameCol}`}>
                    <span className={styles.orgCell}>
                      <OrgMark org={org} />
                      <span>
                        {org.companyHref ? (
                          <Link className={styles.cellMain} href={`${org.companyHref}#spending`} data-row-primary="">
                            {org.name}
                          </Link>
                        ) : (
                          <span className={styles.cellMain}>{org.name}</span>
                        )}
                        <span className={styles.cellSub}>{org.ticker ?? ""}</span>
                      </span>
                    </span>
                  </td>
                  <td data-label="Latest quarter">
                    {last ? (
                      <>
                        {fiscalLabel(last)}
                        <span className={styles.cellSub}>
                          {calendarSpanLabel(last.start, last.end)}
                          {last.derived ? " · derived" : ""}
                        </span>
                      </>
                    ) : (
                      <Missing reason="No quarterly figure: the company files full years only, or quarters cannot be derived" />
                    )}
                  </td>
                  <td className={`${styles.right} ${styles.amountCell}`} data-label="Cash paid">
                    {last ? (
                      <>
                        <span className={styles.num}>{formatMoney(last.value, last.currency)}</span>
                        <span className={styles.cellSub}>
                          <CapexSources sources={last.sources} compact />
                        </span>
                        <AmountBar value={last.value} max={scale.get(last.currency) ?? 0} />
                      </>
                    ) : (
                      <Missing />
                    )}
                  </td>
                  <td className={styles.right} data-label="Last 4 quarters">
                    {contiguous ? (
                      <>
                        <span className={styles.num}>{formatMoney(lastFour.reduce((s, q) => s + q.value, 0), lastFour[0].currency)}</span>
                        <span className={styles.cellSub}>computed</span>
                      </>
                    ) : (
                      <Missing reason="Fewer than four consecutive quarters" />
                    )}
                  </td>
                  <td className={styles.right} data-label="Latest fiscal year">
                    {year ? (
                      <>
                        <span className={styles.num}>{formatMoney(year.value, year.currency)}</span>
                        <span className={styles.cellSub}>
                          FY{year.fiscalYear} ·{" "}
                          <a className={styles.link} href={year.source.url} target="_blank" rel="noopener noreferrer">
                            {year.source.form}
                          </a>
                        </span>
                      </>
                    ) : (
                      <Missing />
                    )}
                  </td>
                  <td className={styles.notesCol} data-label="Line item and notes">
                    {view.lineItem ? <span className={styles.cellSub}>{view.lineItem}</span> : null}
                    {view.notes.length > 0 ? <span className={styles.cellSub}>{view.notes.join(" ")}</span> : null}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
        <p className={styles.note}>
          A derived quarter is a year-to-date figure minus the earlier year-to-date figure from the same fiscal year, because
          10-Q cash flow statements are cumulative. Each value keeps the first figure the company reported; later restatements
          are not applied.{" "}
          <Link className={styles.link} href={methodologyPath()}>
            Method →
          </Link>
        </p>
      </section>
    </CapitalShell>
  );
}
