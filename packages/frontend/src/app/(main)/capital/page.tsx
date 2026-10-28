import type { Metadata } from "next";
import Link from "next/link";
import { TOKSCALE_X_HANDLE } from "@/lib/community";
import {
  capitalUrl,
  companiesPath,
  computePath,
  investorsPath,
  methodologyPath,
  roundsPath,
  spendingPath,
} from "@/lib/capital";
import { formatMoney, formatPartialDate, formatTotal, ledger } from "@/lib/capital/ledger";
import { calendarSpanLabel, fiscalLabel } from "@/lib/capital/ledger/capex";
import { Amount } from "./_components/Amount";
import { CapexBars } from "./_components/CapexBars";
import { CapitalCta } from "./_components/CapitalCta";
import { CapitalShell } from "./_components/CapitalShell";
import { DitherField } from "./_components/DitherField";
import { EventFeed } from "./_components/EventFeed";
import { JsonLd } from "./_components/JsonLd";
import { Reveal } from "./_components/Reveal";
import styles from "./_components/capital.module.css";

const TITLE = "Capital: Funding, Compute Deals and AI Spending | Tokscale";
const DESCRIPTION =
  "Where money moves in the AI economy: funding rounds, compute contracts and cash capex across AI labs, clouds, chips and power. Every number has a source, a date and a status.";

export const metadata: Metadata = {
  title: TITLE,
  description: DESCRIPTION,
  alternates: { canonical: capitalUrl() },
  openGraph: { title: TITLE, description: DESCRIPTION, type: "website", url: capitalUrl(), siteName: "Tokscale" },
  twitter: { card: "summary_large_image", site: TOKSCALE_X_HANDLE, title: TITLE, description: DESCRIPTION },
};

/**
 * A round ceiling just above a value, on one or two significant figures, so the
 * hero field can say plainly what its full height stands for ("the field fills
 * 79% of $300B") instead of comparing this figure with another money type.
 */
function fieldCeiling(value: number): number {
  if (!(value > 0)) return 0;
  const raw = value * 1.18;
  const step = 10 ** Math.floor(Math.log10(raw));
  for (const multiple of [1, 1.5, 2, 2.5, 3, 4, 5, 6, 8, 10]) {
    if (step * multiple >= raw) return step * multiple;
  }
  return step * 10;
}

export default function CapitalPage() {
  const reviewed = ledger.reviewedAt();
  const year = reviewed.slice(0, 4);
  const events = ledger.getEvents();
  const companies = ledger.getCompanies();
  const investors = ledger.getInvestors();
  const equity = ledger.equityYtd(year);
  const usdEquity = equity.find((t) => t.currency === "USD") ?? null;
  const otherEquity = equity.filter((t) => t.currency !== "USD");
  const compute = ledger.computeYear(year);
  const capexQuarter = ledger.capexCalendarQuarter();
  const valuation = ledger.largestValuation();
  const capex = ledger.getCapex();
  const sectors = ledger.getSectors();

  const labRounds = ledger
    .getRounds()
    .filter((e) => e.event.types.includes("equity_round") && e.issuer?.org.categories.includes("ai-lab"));
  const labCompute = ledger
    .getComputeDeals()
    .filter((e) => e.participants.some((p) => p.org.org.categories.includes("ai-lab") && p.roles.some((r) => r === "buyer" || r === "lessee")));
  const leases = ledger.getEvents().filter((e) => e.event.types.some((t) => t === "datacenter_lease" || t === "ppa"));
  const topInvestors = [...investors].sort((a, b) => b.leads - a.leads || b.rounds - a.rounds || a.org.name.localeCompare(b.org.name)).slice(0, 4);

  const equityValue = usdEquity?.value ?? null;
  const ceiling = equityValue === null ? 0 : fieldCeiling(equityValue);
  const share = equityValue !== null && ceiling > 0 ? equityValue / ceiling : 0;

  return (
    <CapitalShell section="overview">
      <JsonLd
        data={{
          "@context": "https://schema.org",
          "@type": "CollectionPage",
          name: "Capital",
          description: DESCRIPTION,
          url: capitalUrl(),
          dateModified: reviewed,
        }}
      />
      <div className={styles.landing}>
        <Reveal />
        <header className={styles.hero}>
          <h1 className={`${styles.display} ${styles.heroTitle}`} data-enter="" style={{ "--i": 0 } as React.CSSProperties}>
            Where money moves in the AI economy.
          </h1>
          <p className={styles.heroLede} data-enter="" style={{ "--i": 1 } as React.CSSProperties}>
            Funding rounds, compute contracts and capital spending across AI labs, clouds, chips and power. Every number has a
            source, a date and a status.
          </p>
          <figure className={styles.heroField}>
            <div className={styles.fieldCopy}>
              <p className={styles.fieldValue}>{usdEquity ? formatTotal(usdEquity) : "—"}</p>
              <p className={styles.fieldNote}>
                {usdEquity ? (
                  <>
                    Equity raised by AI labs in {year} so far: {usdEquity.counted.length} closed{" "}
                    {usdEquity.counted.length === 1 ? "round" : "rounds"}. The field fills {Math.round(share * 100)}% of{" "}
                    {formatMoney(ceiling, "USD")}.
                  </>
                ) : (
                  <>No closed AI-lab equity round is tracked for {year} yet, so there is nothing to fill.</>
                )}
                {otherEquity.map((t) => ` Plus ${formatTotal(t)} in ${t.currency}, not converted.`).join("")}
              </p>
            </div>
            <div className={styles.fieldCanvas}>
              <DitherField share={share} />
            </div>
          </figure>
          <div className={styles.heroFoot}>
            <p className={styles.heroAction}>
              <CapitalCta href={roundsPath()}>See the funding rounds</CapitalCta>
            </p>
            <p className={styles.heroMeta}>
              <span>{companies.length} companies</span>
              <span>{investors.length} investors</span>
              <span>{events.length} events</span>
              <span>Reviewed {formatPartialDate(reviewed)}</span>
            </p>
          </div>
        </header>

        <section className={styles.section} aria-labelledby="money-types">
          <div className={styles.sectionHead}>
            <h2 id="money-types" className={styles.sectionTitle} data-reveal="">
              Four money types
            </h2>
            <p className={styles.sectionNote} data-reveal="">
              Counted on their own terms
            </p>
          </div>
          <div className={styles.moneyStrip} data-ui="" data-reveal="">
            <div className={styles.moneyCard}>
              <p className={styles.moneyLabel}>Equity raised, {year} so far</p>
              <p className={`${styles.moneyValue} ${styles.num}`}>{usdEquity ? formatTotal(usdEquity) : "—"}</p>
              <p className={styles.moneyText}>
                {usdEquity
                  ? `${usdEquity.counted.length} closed equity ${usdEquity.counted.length === 1 ? "round" : "rounds"} by AI labs Tokscale tracks`
                  : "No closed AI-lab rounds tracked this year"}
                {otherEquity.map((t) => ` · plus ${formatTotal(t)} in ${t.currency} (not converted)`).join("")}
              </p>
              <p className={styles.more}>
                <Link className={styles.link} href={roundsPath()}>
                  Rounds
                </Link>
              </p>
            </div>
            <div className={styles.moneyCard}>
              <p className={styles.moneyLabel}>Compute deals, {year}</p>
              {compute.summed.ok && compute.summed.total.value !== null ? (
                <>
                  <p className={`${styles.moneyValue} ${styles.num}`}>{formatTotal(compute.summed.total)}</p>
                  <p className={styles.moneyText}>
                    {compute.summed.total.counted.length} stated contract values, multi-year totals, not annual spend
                  </p>
                </>
              ) : (
                <>
                  <p className={`${styles.moneyValue} ${styles.num}`}>{compute.deals.length} deals</p>
                  <p className={styles.moneyText}>
                    {compute.largest ? (
                      <>
                        Largest stated value <Amount view={compute.largest} />
                        {compute.largest.event ? ` (${ledger.eventView(compute.largest.event).partiesText})` : ""}. Values use
                        different measures and bases, so they are not added.
                      </>
                    ) : (
                      "No stated contract values this year"
                    )}
                  </p>
                </>
              )}
              <p className={styles.more}>
                <Link className={styles.link} href={computePath()}>
                  Compute
                </Link>
              </p>
            </div>
            <div className={styles.moneyCard}>
              <p className={styles.moneyLabel}>Cash capex, {capexQuarter ? capexQuarter.label : "latest quarter"}</p>
              <p className={`${styles.moneyValue} ${styles.num}`}>{capexQuarter ? formatTotal(capexQuarter.total) : "—"}</p>
              <p className={styles.moneyText}>
                {capexQuarter
                  ? `${capexQuarter.entries.length} companies, cash paid for property and equipment, ${calendarSpanLabel(capexQuarter.entries[0].quarter.start, capexQuarter.entries[0].quarter.end)}${capexQuarter.derivedCount > 0 ? `; ${capexQuarter.derivedCount} derived from year-to-date filings` : ""}`
                  : "Not enough companies have reported the same calendar quarter yet"}
              </p>
              <p className={styles.more}>
                <Link className={styles.link} href={spendingPath()}>
                  Spending
                </Link>
              </p>
            </div>
            <div className={styles.moneyCard}>
              <p className={styles.moneyLabel}>Largest valuation</p>
              <p className={`${styles.moneyValue} ${styles.num}`}>{valuation ? valuation.amount.short : "—"}</p>
              <p className={styles.moneyText}>
                {valuation
                  ? `${valuation.event.issuer?.name ?? valuation.event.partiesText}, post-money, ${formatPartialDate(valuation.event.date)} (${valuation.amount.statusLabel.toLowerCase()} by the company)`
                  : "No valuation tracked"}
              </p>
              <p className={styles.more}>
                {valuation ? (
                  <Link className={styles.link} href={valuation.event.href}>
                    {valuation.event.title}
                  </Link>
                ) : null}
              </p>
            </div>
          </div>
          <p className={styles.moneyNote} data-reveal="">
            These four numbers are different money types. They are never added together.{" "}
            <Link className={styles.link} href={methodologyPath()}>
              How we count
            </Link>
          </p>
        </section>

        <section className={styles.section} aria-labelledby="latest-events">
          <div className={styles.sectionHead}>
            <h2 id="latest-events" className={styles.sectionTitle} data-reveal="">
              Latest events
            </h2>
            <p className={styles.sectionNote} data-reveal="">
              <Link className={styles.link} href={roundsPath()}>
                All rounds
              </Link>{" "}
              ·{" "}
              <Link className={styles.link} href={computePath()}>
                All compute deals
              </Link>
            </p>
          </div>
          <div data-ui="" data-reveal="">
            <EventFeed events={events.slice(0, 6)} variant="ledger" />
          </div>
        </section>

        <section className={styles.section} aria-labelledby="stack">
          <div className={styles.sectionHead}>
            <h2 id="stack" className={styles.sectionTitle} data-reveal="">
              Follow the money down the stack
            </h2>
            <p className={styles.sectionNote} data-reveal="">
              Counts of events and companies Tokscale tracks, not market totals
            </p>
          </div>
          <ul className={styles.stack2} data-reveal="">
            <li>
              <span>
                Investors <span className={styles.flowArrow}>→ equity →</span> AI labs
              </span>
              <Link className={styles.link} href={roundsPath()}>
                {labRounds.length} rounds
              </Link>
            </li>
            <li>
              <span>
                AI labs <span className={styles.flowArrow}>→ contracts →</span> clouds and chips
              </span>
              <Link className={styles.link} href={computePath()}>
                {labCompute.length} deals
              </Link>
            </li>
            <li>
              <span>
                Clouds <span className={styles.flowArrow}>→ capex →</span> chips and data centers
              </span>
              <Link className={styles.link} href={spendingPath()}>
                {capex.length} companies
              </Link>
            </li>
            <li>
              <span>
                Data centers <span className={styles.flowArrow}>→ leases and power →</span> landlords and utilities
              </span>
              <Link className={styles.link} href={computePath()}>
                {leases.length} {leases.length === 1 ? "lease" : "leases"}
              </Link>
            </li>
          </ul>
        </section>

        <section className={styles.section} aria-labelledby="capex">
          <div className={styles.sectionHead}>
            <h2 id="capex" className={styles.sectionTitle} data-reveal="">
              Cash paid for property and equipment
            </h2>
            <p className={styles.sectionNote} data-reveal="">
              Latest quarter per company, from SEC filings
            </p>
          </div>
          {capex.length > 0 ? (
            <>
              <ul className={styles.multiples} data-ui="" data-reveal="">
                {capex
                  .filter(({ view }) => view.quarters.length > 0)
                  .slice(0, 6)
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
                          <span>
                            {calendarSpanLabel(last.start, last.end)}
                            {last.derived ? " · derived" : ""}
                          </span>
                        </p>
                      </li>
                    );
                  })}
              </ul>
              <p className={`${styles.note} ${styles.legendRow}`}>
                <span className={styles.legendKey}>
                  <span className={styles.keySolid} aria-hidden="true" /> Reported quarter
                </span>
                <span className={styles.legendKey}>
                  <span className={styles.keyHollow} aria-hidden="true" /> Derived: year to date minus the earlier year to date
                </span>
              </p>
              <p className={styles.note}>
                <Link className={styles.link} href={spendingPath()}>
                  All companies, fiscal labels and filings
                </Link>
              </p>
            </>
          ) : (
            <p className={styles.empty}>Capex data is not loaded yet.</p>
          )}
        </section>

        <section className={styles.section} aria-labelledby="investors">
          <div className={styles.sectionHead}>
            <h2 id="investors" className={styles.sectionTitle} data-reveal="">
              Investors in tracked rounds
            </h2>
            <p className={styles.sectionNote} data-reveal="">
              <Link className={styles.link} href={investorsPath()}>
                All {investors.length} investors
              </Link>
            </p>
          </div>
          <div data-ui="" data-reveal="">
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
                </tr>
              </thead>
              <tbody>
                {topInvestors.map((row) => (
                  <tr key={row.org.id}>
                    <td className={styles.full}>
                      <Link className={styles.cellMain} href={row.org.investorHref ?? investorsPath()}>
                        {row.org.name}
                      </Link>
                    </td>
                    <td data-label="Type">{row.org.investorTypeLabels.join(", ")}</td>
                    <td className={`${styles.right} ${styles.num}`} data-label="Rounds and deals">
                      {row.rounds}
                    </td>
                    <td className={`${styles.right} ${styles.num}`} data-label="Led or co-led">
                      {row.leads}
                    </td>
                    <td data-label="Latest">
                      {row.latest ? (
                        <>
                          <Link href={row.latest.href}>{row.latest.title}</Link>
                          <span className={styles.cellSub}>{formatPartialDate(row.latest.date)}</span>
                        </>
                      ) : (
                        "—"
                      )}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </section>

        <section className={styles.section} aria-labelledby="sectors">
          <div className={styles.sectionHead}>
            <h2 id="sectors" className={styles.sectionTitle} data-reveal="">
              Sectors
            </h2>
            <p className={styles.sectionNote} data-reveal="">
              <Link className={styles.link} href={companiesPath()}>
                All {companies.length} companies
              </Link>
            </p>
          </div>
          <ul className={styles.chips} data-reveal="">
            {sectors.map((sector) => (
              <li key={sector.slug}>
                <Link href={sector.href}>
                  {sector.name} <span className={styles.muted}>{sector.companies.length}</span>
                </Link>
              </li>
            ))}
          </ul>
        </section>

        <section className={styles.section} aria-labelledby="method">
          <div className={styles.sectionHead}>
            <h2 id="method" className={styles.sectionTitle} data-reveal="">
              How we count
            </h2>
            <p className={styles.sectionNote} data-reveal="">
              <Link className={styles.link} href={methodologyPath()}>
                Money types, statuses, coverage and corrections
              </Link>
            </p>
          </div>
          <p className={styles.moneyNote} data-reveal="">
            Totals exist only inside one money type, one currency and one basis. Money already counted in a later round is linked
            to it and counted once. Press-reported numbers are labeled with the outlet and never enter totals.
          </p>
        </section>
      </div>
    </CapitalShell>
  );
}
