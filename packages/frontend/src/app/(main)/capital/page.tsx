import type { Metadata } from "next";
import Link from "next/link";
import { TOKSCALE_X_HANDLE } from "@/lib/community";
import { capitalUrl, companiesPath, computePath, methodologyPath, roundsPath, spendingPath } from "@/lib/capital";
import {
  formatMoney,
  formatPartialDate,
  formatTotal,
  ledger,
} from "@/lib/capital/ledger";
import { calendarSpanLabel, fiscalLabel } from "@/lib/capital/ledger/capex";
import { Amount } from "./_components/Amount";
import { CapexBars } from "./_components/CapexBars";
import { CapitalShell } from "./_components/CapitalShell";
import { EventFeed } from "./_components/EventFeed";
import { JsonLd } from "./_components/JsonLd";
import styles from "./_components/capital.module.css";
import { Eyebrow } from "@/components/ui/Eyebrow";

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
      <header className={styles.hero}>
        <Eyebrow>Capital</Eyebrow>
        <h1 className={`${styles.display} ${styles.heroTitle}`}>Where money moves in the AI economy.</h1>
        <p className={styles.heroLede}>
          Funding rounds, compute contracts and capital spending across AI labs, clouds, chips and power. Every number has a
          source, a date and a status.
        </p>
        <p className={styles.heroMeta}>
          {companies.length} companies · {investors.length} investors · {events.length} events · Reviewed {formatPartialDate(reviewed)}
        </p>
      </header>

      <section aria-labelledby="money-types">
        <h2 id="money-types" className={styles.srOnly}>
          Four money types
        </h2>
        <div className={styles.moneyCards}>
          <div className={styles.moneyCard}>
            <Eyebrow>Equity · {year} So Far</Eyebrow>
            <p className={styles.moneyValue}>{usdEquity ? formatTotal(usdEquity) : "—"}</p>
            <p className={styles.cellSub}>
              {usdEquity
                ? `${usdEquity.counted.length} closed equity ${usdEquity.counted.length === 1 ? "round" : "rounds"} by AI labs Tokscale tracks`
                : "No closed AI-lab rounds tracked this year"}
              {otherEquity.map((t) => ` · plus ${formatTotal(t)} in ${t.currency} (not converted)`).join("")}
            </p>
            <p className={styles.more}>
              <Link className={styles.link} href={roundsPath()}>
                Rounds →
              </Link>
            </p>
          </div>
          <div className={styles.moneyCard}>
            <Eyebrow>Compute Deals · {year}</Eyebrow>
            {compute.summed.ok && compute.summed.total.value !== null ? (
              <>
                <p className={styles.moneyValue}>{formatTotal(compute.summed.total)}</p>
                <p className={styles.cellSub}>
                  {compute.summed.total.counted.length} stated contract values, multi-year totals, not annual spend
                </p>
              </>
            ) : (
              <>
                <p className={styles.moneyValue}>{compute.deals.length} deals</p>
                <p className={styles.cellSub}>
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
                Compute →
              </Link>
            </p>
          </div>
          <div className={styles.moneyCard}>
            <Eyebrow>Cash Capex · {capexQuarter ? capexQuarter.label : "Latest Quarter"}</Eyebrow>
            <p className={styles.moneyValue}>{capexQuarter ? formatTotal(capexQuarter.total) : "—"}</p>
            <p className={styles.cellSub}>
              {capexQuarter
                ? `${capexQuarter.entries.length} companies, cash paid for property and equipment, ${calendarSpanLabel(capexQuarter.entries[0].quarter.start, capexQuarter.entries[0].quarter.end)}${capexQuarter.derivedCount > 0 ? `; ${capexQuarter.derivedCount} derived from year-to-date filings` : ""}`
                : "Not enough companies have reported the same calendar quarter yet"}
            </p>
            <p className={styles.more}>
              <Link className={styles.link} href={spendingPath()}>
                Spending →
              </Link>
            </p>
          </div>
          <div className={styles.moneyCard}>
            <Eyebrow>Largest Valuation</Eyebrow>
            <p className={styles.moneyValue}>{valuation ? valuation.amount.short : "—"}</p>
            <p className={styles.cellSub}>
              {valuation
                ? `${valuation.event.issuer?.name ?? valuation.event.partiesText}, post-money, ${formatPartialDate(valuation.event.date)} (${valuation.amount.statusLabel.toLowerCase()} by the company)`
                : "No valuation tracked"}
            </p>
            <p className={styles.more}>
              {valuation ? (
                <Link className={styles.link} href={valuation.event.href}>
                  {valuation.event.title} →
                </Link>
              ) : null}
            </p>
          </div>
        </div>
        <p className={styles.moneyNote}>
          These four numbers are different money types. They are never added together.{" "}
          <Link className={styles.link} href={methodologyPath()}>
            How we count →
          </Link>
        </p>
      </section>

      <section className={styles.section} style={{ marginTop: 40 }} aria-labelledby="latest-events">
        <div className={styles.sectionHead}>
          <h2 id="latest-events" className={`${styles.display} ${styles.sectionTitle}`}>
            Latest events
          </h2>
          <p className={styles.sectionNote}>
            <Link className={styles.link} href={roundsPath()}>
              All rounds →
            </Link>{" "}
            ·{" "}
            <Link className={styles.link} href={computePath()}>
              All compute deals →
            </Link>
          </p>
        </div>
        <EventFeed events={events.slice(0, 8)} />
      </section>

      <section className={styles.section} aria-label="The stack and capital spending">
        <div className={`${styles.grid} ${styles.gridEven}`}>
          <div>
            <h2 className={`${styles.display} ${styles.sectionTitle}`} style={{ marginBottom: 16 }}>
              Follow the money down the stack
            </h2>
            <ul className={styles.stack2}>
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
            <p className={styles.note}>Counts of events and companies Tokscale tracks, not market totals.</p>
          </div>
          <div>
            <h2 className={`${styles.display} ${styles.sectionTitle}`} style={{ marginBottom: 16 }}>
              Cash paid for property and equipment
            </h2>
            {capex.length > 0 ? (
              <>
                <ul className={`${styles.multiples} ${styles.multiplesTight}`}>
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
                <p className={styles.note} style={{ marginTop: 8 }}>
                  <Link className={styles.link} href={spendingPath()}>
                    All companies, fiscal labels and filings →
                  </Link>
                </p>
              </>
            ) : (
              <p className={styles.empty}>Capex data is not loaded yet.</p>
            )}
          </div>
        </div>
      </section>

      <section className={styles.section} aria-labelledby="sectors">
        <div className={styles.sectionHead}>
          <h2 id="sectors" className={`${styles.display} ${styles.sectionTitle}`}>
            Sectors
          </h2>
          <p className={styles.sectionNote}>
            <Link className={styles.link} href={companiesPath()}>
              All {companies.length} companies →
            </Link>
          </p>
        </div>
        <ul className={styles.chips}>
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
          <h2 id="method" className={`${styles.display} ${styles.sectionTitle}`}>
            How we count
          </h2>
          <p className={styles.sectionNote}>
            <Link className={styles.link} href={methodologyPath()}>
              Money types, statuses, coverage and corrections →
            </Link>
          </p>
        </div>
        <p className={styles.note} style={{ marginTop: 0 }}>
          Totals exist only inside one money type, one currency and one basis. Money already counted in a later round is
          linked to it and counted once. Press-reported numbers are labeled with the outlet and never enter totals.
        </p>
      </section>
    </CapitalShell>
  );
}
