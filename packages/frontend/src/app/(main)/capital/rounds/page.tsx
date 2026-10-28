import type { Metadata } from "next";
import Link from "next/link";
import { TOKSCALE_X_HANDLE } from "@/lib/community";
import { capitalPath, methodologyPath, roundsPath, roundsUrl } from "@/lib/capital";
import { EQUITY_MEASURES, financingAmount, formatPartialDate, ledger, type EventView, type Measure } from "@/lib/capital/ledger";
import { Amount, AmountBar, Missing, maxByCurrency } from "../_components/Amount";
import { Breadcrumbs } from "../_components/Breadcrumbs";
import { CapitalShell } from "../_components/CapitalShell";
import { EventFeed } from "../_components/EventFeed";
import { JsonLd } from "../_components/JsonLd";
import { SourceLink } from "../_components/SourceLink";
import styles from "../_components/capital.module.css";

const TITLE = "AI Funding Rounds, Debt and IPOs | Tokscale";
const DESCRIPTION =
  "Funding rounds, debt financings, IPOs and Form D filings of AI labs, clouds and chip companies, each with amounts, investors and the primary source.";

export const metadata: Metadata = {
  title: TITLE,
  description: DESCRIPTION,
  alternates: { canonical: roundsUrl() },
  openGraph: { title: TITLE, description: DESCRIPTION, type: "website", url: roundsUrl(), siteName: "Tokscale" },
  twitter: { card: "summary_large_image", site: TOKSCALE_X_HANDLE, title: TITLE, description: DESCRIPTION },
};

const FILTERS: Array<{ key: string; label: string; measures?: readonly Measure[]; match: (e: EventView) => boolean }> = [
  { key: "", label: "All", match: () => true },
  { key: "equity", label: "Equity rounds", measures: EQUITY_MEASURES, match: (e) => e.event.types.includes("equity_round") || e.event.types.includes("strategic_investment") || Boolean(financingAmount(e, EQUITY_MEASURES)) },
  { key: "debt", label: "Debt", measures: ["debt_raised"], match: (e) => e.event.types.includes("debt_facility") || Boolean(financingAmount(e, ["debt_raised"])) },
  { key: "ipo", label: "IPOs and listings", measures: ["ipo_proceeds"], match: (e) => e.event.types.includes("ipo") || e.event.types.includes("registration") || Boolean(financingAmount(e, ["ipo_proceeds"])) },
  { key: "form-d", label: "Form D", measures: ["form_d_amount_sold", "form_d_offering_amount"], match: (e) => e.event.types.includes("form_d") || Boolean(financingAmount(e, ["form_d_amount_sold", "form_d_offering_amount"])) },
];

interface PageProps {
  searchParams: Promise<{ type?: string | string[] }>;
}

export default async function RoundsPage({ searchParams }: PageProps) {
  const raw = (await searchParams).type;
  const key = (Array.isArray(raw) ? raw[0] : raw) ?? "";
  const filter = FILTERS.find((f) => f.key === key) ?? FILTERS[0];
  const all = ledger.getFinancings();
  const rows = all.filter(filter.match);
  const acquisitions = ledger.getDeals().filter((e) => e.event.kind === "acquisition");
  // Bars are scaled inside one currency, so a euro figure never sits against a dollar one.
  const scale = maxByCurrency(
    rows.map((e) => {
      const main = financingAmount(e, filter.measures);
      return { value: main?.amount.value ?? null, currency: main?.amount.currency ?? "" };
    })
  );

  return (
    <CapitalShell section="rounds">
      <JsonLd
        data={{
          "@context": "https://schema.org",
          "@type": "ItemList",
          name: "AI financing events Tokscale tracks",
          url: roundsUrl(),
          numberOfItems: all.length,
          itemListElement: all.map((e, index) => ({ "@type": "ListItem", position: index + 1, name: e.title, url: roundsUrl().replace(/\/capital\/rounds$/, e.href) })),
        }}
      />
      <Breadcrumbs items={[{ label: "Capital", href: capitalPath() }, { label: "Rounds" }]} />
      <header className={styles.pageHead}>
        <h1 className={`${styles.display} ${styles.pageTitle}`}>Funding rounds, debt and IPOs</h1>
        <p className={styles.lede}>
          How AI companies raise money. Each event has one page with its amounts, investors and the sentence each number comes
          from.
        </p>
      </header>

      <nav aria-label="Financing type">
        <ul className={styles.views}>
          {FILTERS.map((f) => (
            <li key={f.key || "all"}>
              <Link href={f.key ? `${roundsPath()}?type=${f.key}` : roundsPath()} aria-current={f === filter ? "true" : undefined} scroll={false}>
                {f.label} {all.filter(f.match).length}
              </Link>
            </li>
          ))}
        </ul>
      </nav>

      <table className={`${styles.ledgerTable} ${styles.ledgerStack}`}>
        <thead>
          <tr>
            <th scope="col">Date</th>
            <th scope="col">Event</th>
            <th scope="col">Type</th>
            <th scope="col" className={`${styles.right} ${styles.amountCell}`}>
              Amount
            </th>
            <th scope="col" className={styles.right}>
              Post-money
            </th>
            <th scope="col">Leads</th>
            <th scope="col">Status</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((e) => {
            // The selected financing category can differ from a mixed event's compute headline.
            const main = financingAmount(e, filter.measures);
            const leads = e.participants.filter((p) => p.roles.includes("lead_investor") || p.roles.includes("co_lead"));
            return (
              <tr key={e.id} data-row-link="">
                <td className={styles.nowrap} data-label="Date">
                  {formatPartialDate(e.date)}
                </td>
                <td className={styles.full}>
                  <Link className={styles.cellMain} href={e.href} data-row-primary="">
                    {e.title}
                  </Link>
                  <span className={styles.cellSub}>{e.issuer?.name ?? e.partiesText}</span>
                </td>
                <td data-label="Type">{e.typeLabels.join(", ")}</td>
                <td className={`${styles.right} ${styles.amountCell} ${main ? "" : styles.emptyStacked}`} data-label="Amount">
                  {main ? (
                    <>
                      <Amount view={main} />
                      <span className={styles.cellSub}>{main.amount.label ?? ""}</span>
                      {main.skipped === "included_in" ? <span className={styles.cellSub}>counted in a later event</span> : null}
                      <AmountBar value={main.amount.value} max={scale.get(main.amount.currency) ?? 0} />
                    </>
                  ) : (
                    <Missing reason="No amount stated in the source" />
                  )}
                </td>
                <td className={`${styles.right} ${e.valuation ? "" : styles.emptyStacked}`} data-label="Post-money">
                  {e.valuation ? <Amount view={e.valuation} /> : <Missing />}
                </td>
                <td className={leads.length > 0 ? undefined : styles.emptyStacked} data-label="Leads">
                  {leads.length > 0 ? leads.map((l) => l.org.displayName).join(", ") : <Missing />}
                </td>
                <td data-label="Status">
                  <span className={styles.statusText}>{main?.statusLabel ?? e.statusLabel}</span> <SourceLink source={main?.source ?? e.sources[0]} />
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
      {rows.length === 0 ? <p className={styles.empty}>No event of this type yet.</p> : null}
      <p className={styles.note}>
        Amounts keep their own type: equity, debt, IPO proceeds and Form D amounts are never added together. Money that a later
        round already counts is marked on the event page.{" "}
        <Link className={styles.link} href={methodologyPath()}>
          Method →
        </Link>
      </p>

      {acquisitions.length > 0 ? (
        <section className={styles.section} style={{ marginTop: 40 }} id="acquisitions" aria-labelledby="acquisitions-title">
          <div className={styles.sectionHead}>
            <h2 id="acquisitions-title" className={`${styles.display} ${styles.sectionTitle}`}>
              Mergers and acquisitions
            </h2>
          </div>
          <EventFeed events={acquisitions} />
        </section>
      ) : null}
    </CapitalShell>
  );
}
