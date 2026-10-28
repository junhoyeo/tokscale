import type { Metadata } from "next";
import Link from "next/link";
import { InstantTooltip } from "@/components/ui/InstantTooltip";
import { TOKSCALE_X_HANDLE } from "@/lib/community";
import { capitalPath, computePath, computeUrl, methodologyPath } from "@/lib/capital";
import { QUANTITY_UNITS, formatPartialDate, ledger, tryQuantityTotal, type EventView, type QuantityMeasure } from "@/lib/capital/ledger";
import { Amount, AmountBar, Missing, maxByCurrency } from "../_components/Amount";
import { Breadcrumbs } from "../_components/Breadcrumbs";
import { CapacityTotal } from "../_components/CapacityTotal";
import { CapitalShell } from "../_components/CapitalShell";
import { JsonLd } from "../_components/JsonLd";
import { SourceLink } from "../_components/SourceLink";
import styles from "../_components/capital.module.css";

const TITLE = "AI Compute Deals: Cloud Contracts, Data-Center Leases and Chips | Tokscale";
const DESCRIPTION =
  "Compute and infrastructure contracts between AI labs, clouds, chip makers and data-center owners, in dollars and in MW or GW, from SEC filings and company statements.";

export const metadata: Metadata = {
  title: TITLE,
  description: DESCRIPTION,
  alternates: { canonical: computeUrl() },
  openGraph: { title: TITLE, description: DESCRIPTION, type: "website", url: computeUrl(), siteName: "Tokscale" },
  twitter: { card: "summary_large_image", site: TOKSCALE_X_HANDLE, title: TITLE, description: DESCRIPTION },
};

const FILTERS: Array<{ key: string; label: string; match: (e: EventView) => boolean }> = [
  { key: "", label: "All", match: () => true },
  { key: "cloud", label: "Cloud and compute", match: (e) => e.event.types.some((t) => t === "compute_contract" || t === "compute_commitment") },
  { key: "lease", label: "Data-center leases", match: (e) => e.event.types.includes("datacenter_lease") },
  { key: "chips", label: "Chip supply", match: (e) => e.event.types.includes("chip_supply") },
  { key: "assets", label: "Asset sales and joint ventures", match: (e) => e.event.types.some((t) => t === "asset_sale" || t === "joint_venture") },
];

const sides = (e: EventView) => ({
  buyers: e.participants.filter((p) => p.roles.some((r) => r === "buyer" || r === "lessee" || r === "acquirer")),
  providers: e.participants.filter((p) => p.roles.some((r) => r === "provider" || r === "lessor" || r === "seller")),
  partners: e.participants.filter((p) => p.roles.includes("partner")),
});

interface PageProps {
  searchParams: Promise<{ type?: string | string[] }>;
}

export default async function ComputePage({ searchParams }: PageProps) {
  const raw = (await searchParams).type;
  const key = (Array.isArray(raw) ? raw[0] : raw) ?? "";
  const filter = FILTERS.find((f) => f.key === key) ?? FILTERS[0];
  const all = ledger.getComputeDeals();
  const rows = all.filter(filter.match);
  // Bars are scaled inside one currency; contract values keep their stated basis.
  const scale = maxByCurrency(
    rows.map((e) => {
      const value = e.amounts.find(
        (a) => a.skipped === null && ["contract_value", "compute_commitment", "purchase_price"].includes(a.amount.measure)
      );
      return { value: value?.amount.value ?? null, currency: value?.amount.currency ?? "" };
    })
  );

  // Per buyer: count and largest stated value. Capacity is summed only inside one measure.
  const buyers = new Map<string, { name: string; href: string | null; deals: EventView[] }>();
  for (const e of all) {
    for (const b of sides(e).buyers) {
      const entry = buyers.get(b.org.id) ?? { name: b.org.name, href: b.org.companyHref, deals: [] };
      entry.deals.push(e);
      buyers.set(b.org.id, entry);
    }
  }
  const capacityBy = (deals: EventView[], measure: QuantityMeasure) =>
    tryQuantityTotal(
      deals.flatMap((d) => d.quantities.map((q) => q.quantity)),
      measure
    );

  return (
    <CapitalShell section="compute">
      <JsonLd
        data={{
          "@context": "https://schema.org",
          "@type": "ItemList",
          name: "AI compute and infrastructure deals Tokscale tracks",
          url: computeUrl(),
          numberOfItems: all.length,
          itemListElement: all.map((e, index) => ({ "@type": "ListItem", position: index + 1, name: e.title, url: computeUrl().replace(/\/capital\/compute$/, e.href) })),
        }}
      />
      <Breadcrumbs items={[{ label: "Capital", href: capitalPath() }, { label: "Compute" }]} />
      <header className={styles.pageHead}>
        <h1 className={`${styles.display} ${styles.pageTitle}`}>Compute and infrastructure deals</h1>
        <p className={styles.lede}>
          Contracts for cloud capacity, data-center leases, chips and power among companies Tokscale tracks. Contract values
          are stated totals over multi-year terms. They are not annual spend.
        </p>
      </header>

      <nav aria-label="Deal type">
        <ul className={styles.views}>
          {FILTERS.map((f) => (
            <li key={f.key || "all"}>
              <Link href={f.key ? `${computePath()}?type=${f.key}` : computePath()} aria-current={f === filter ? "true" : undefined} scroll={false}>
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
            <th scope="col">Deal</th>
            <th scope="col">Buyer or tenant</th>
            <th scope="col">Provider</th>
            <th scope="col" className={`${styles.right} ${styles.amountCell}`}>
              Value (basis)
            </th>
            <th scope="col" className={styles.right}>
              Capacity
            </th>
            <th scope="col">Source</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((e) => {
            const { buyers: b, providers: pr, partners } = sides(e);
            const value = e.amounts.find(
              (a) => a.skipped === null && ["contract_value", "compute_commitment", "purchase_price"].includes(a.amount.measure)
            );
            const alternative = e.amounts.find((a) => a.skipped === "alternative");
            const capacity = e.quantities.filter((q) => q.quantity.measure !== "shares" && q.quantity.measure !== "investors");
            return (
              <tr key={e.id} data-row-link="">
                <td className={styles.nowrap} data-label="Date">
                  {formatPartialDate(e.date)}
                </td>
                <td className={styles.full}>
                  <Link className={styles.cellMain} href={e.href} data-row-primary="">
                    {e.title}
                  </Link>
                  <span className={styles.cellSub}>
                    {e.typeLabels.join(", ")}
                    {partners.length > 0 ? ` · with ${partners.map((p) => `${p.org.displayName}${p.note ? ` (${p.note.toLowerCase()})` : ""}`).join(", ")}` : ""}
                  </span>
                </td>
                <td data-label="Buyer or tenant">{b.map((p) => p.org.displayName).join(", ") || <Missing />}</td>
                <td data-label="Provider">{pr.map((p) => p.org.displayName).join(", ") || <Missing />}</td>
                <td className={`${styles.right} ${styles.amountCell} ${value ? "" : styles.emptyStacked}`} data-label="Value">
                  {value ? (
                    <>
                      <Amount view={value} />
                      {value.term ? <span className={styles.cellSub}>{value.term}</span> : null}
                      {alternative ? <span className={styles.cellSub}>{alternative.short} with options</span> : null}
                      <AmountBar value={value.amount.value} max={scale.get(value.amount.currency) ?? 0} />
                    </>
                  ) : (
                    <Missing reason="No dollar value stated in the source" />
                  )}
                </td>
                <td className={`${styles.right} ${styles.num} ${capacity.length > 0 ? "" : styles.emptyStacked}`} data-label="Capacity">
                  {capacity.length > 0 ? (
                    capacity.map((q) => (
                      <InstantTooltip key={q.quantity.id} style={{ display: "block" }} content={QUANTITY_UNITS[q.quantity.measure].label}>
                        {q.short}
                      </InstantTooltip>
                    ))
                  ) : (
                    <Missing reason="No capacity stated in the source" />
                  )}
                </td>
                <td data-label="Source">
                  <SourceLink source={value?.source ?? capacity[0]?.source ?? e.sources[0]} />
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
      <p className={styles.note}>
        ≈ about · ≤ up to · &gt; more than · ≥ at least. Values are what the source states, in its currency and basis.
        Tokscale does not annualize them or add different bases together.{" "}
        <Link className={styles.link} href={methodologyPath()}>
          Method →
        </Link>
      </p>

      <section className={styles.section} style={{ marginTop: 40 }} aria-labelledby="by-buyer">
        <div className={styles.sectionHead}>
          <h2 id="by-buyer" className={`${styles.display} ${styles.sectionTitle}`}>
            By buyer
          </h2>
          <p className={styles.sectionNote}>Capacity adds up only within one measure: critical IT MW and GW of chips stay apart</p>
        </div>
        <table className={`${styles.ledgerTable} ${styles.ledgerStack}`}>
          <thead>
            <tr>
              <th scope="col">Buyer or tenant</th>
              <th scope="col" className={styles.right}>
                Deals
              </th>
              <th scope="col" className={styles.right}>
                Largest stated value
              </th>
              <th scope="col" className={styles.right}>
                Critical IT load
              </th>
              <th scope="col" className={styles.right}>
                Capacity in GW
              </th>
            </tr>
          </thead>
          <tbody>
            {[...buyers.entries()]
              .sort((a, b) => b[1].deals.length - a[1].deals.length || a[1].name.localeCompare(b[1].name))
              .map(([id, entry]) => {
                const values = entry.deals.flatMap((d) =>
                  d.amounts.filter((a) => a.skipped === null && (a.amount.measure === "contract_value" || a.amount.measure === "compute_commitment") && a.amount.value !== null && a.amount.status !== "reported")
                );
                const largest = values.sort((a, b) => (b.amount.value ?? 0) - (a.amount.value ?? 0))[0];
                const mw = capacityBy(entry.deals, "it_load_mw");
                const gw = capacityBy(entry.deals, "capacity_gw");
                return (
                  <tr key={id} data-row-link={entry.href ? "" : undefined}>
                    <td className={styles.full}>
                      {entry.href ? (
                        <Link className={styles.cellMain} href={entry.href} data-row-primary="">
                          {entry.name}
                        </Link>
                      ) : (
                        <span className={styles.cellMain}>{entry.name}</span>
                      )}
                    </td>
                    <td className={`${styles.right} ${styles.num}`} data-label="Deals">
                      {entry.deals.length}
                    </td>
                    <td className={styles.right} data-label="Largest stated value">
                      {largest ? <Amount view={largest} /> : <Missing reason="No stated value" />}
                    </td>
                    <td className={styles.right} data-label="Critical IT load">
                      <CapacityTotal result={mw} unit="MW" />
                    </td>
                    <td className={styles.right} data-label="Capacity in GW">
                      <CapacityTotal result={gw} unit="GW" />
                    </td>
                  </tr>
                );
              })}
          </tbody>
        </table>
      </section>
    </CapitalShell>
  );
}
