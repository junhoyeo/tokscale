import type { Metadata } from "next";
import Link from "next/link";
import { notFound } from "next/navigation";
import { ModelMark } from "@/components/explore/ModelMark";
import { TOKSCALE_X_HANDLE } from "@/lib/community";
import { absoluteUrl, capitalPath, companiesPath, companyUrl, edgarFilingsUrl, methodologyPath } from "@/lib/capital";
import {
  CATEGORY_LABELS,
  QUANTITY_UNITS,
  ROLE_LABELS,
  EQUITY_MEASURES,
  financingAmount,
  financingLabel,
  formatMoney,
  formatPartialDate,
  formatTotal,
  ledger,
  type CompanyProfile,
  type EventView,
  type OrgView,
} from "@/lib/capital/ledger";
import { calendarSpanLabel, fiscalLabel } from "@/lib/capital/ledger/capex";
import { loadUsageSnapshot, usageForLabs, type LabUsage } from "@/lib/capital/ledger/usage";
import { Amount, AmountBar, Missing, maxByCurrency } from "../../_components/Amount";
import { Breadcrumbs } from "../../_components/Breadcrumbs";
import { CapexTable } from "../../_components/CapexTable";
import { CapexSources } from "../../_components/CapexSources";
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
/** Tokscale usage on lab profiles refreshes with the explore rollups. */
export const revalidate = 600;

export function generateStaticParams() {
  return ledger.getCompanies().map((c) => ({ slug: c.slug }));
}

const SEO_SUFFIX: Record<CompanyProfile["template"], string> = {
  lab: "Funding Rounds, Valuation, Investors and Compute Deals",
  app: "Funding Rounds, Valuation, Investors and Compute Deals",
  neocloud: "Customer Contracts, Capex and Financing",
  cloud: "Compute Contracts, Capex and Investors",
  hyperscaler: "AI Capex, Compute Deals and AI Investments",
  chips: "AI Deals, Capex and Investments",
  power: "AI Data-Center Leases, Power Deals and Capex",
};

export async function generateMetadata({ params }: PageProps): Promise<Metadata> {
  const { slug } = await params;
  const profile = ledger.getCompany(slug);
  if (!profile) return { title: "Not found | Tokscale", robots: { index: false } };
  const title = `${profile.org.name}: ${SEO_SUFFIX[profile.template]} | Tokscale`;
  const description = `${profile.org.org.description} ${profile.timeline.length} tracked ${profile.timeline.length === 1 ? "event" : "events"} with sources: funding, compute deals${profile.capex ? ", cash capex by quarter" : ""}.`;
  const canonical = companyUrl(slug);
  return {
    title,
    description,
    alternates: { canonical },
    robots: profile.indexable ? undefined : { index: false, follow: true },
    openGraph: { title, description, type: "profile", url: canonical, siteName: "Tokscale" },
    twitter: { card: "summary", site: TOKSCALE_X_HANDLE, title, description },
  };
}

function Fact({ label, value, sub }: { label: string; value: React.ReactNode; sub?: React.ReactNode }) {
  return (
    <div className={styles.stat}>
      <dt className={styles.statLabel}>{label}</dt>
      <dd className={styles.statValue}>{value}</dd>
      {sub ? <dd className={`${styles.statSub} ${styles.factSub}`}>{sub}</dd> : null}
    </div>
  );
}

function HeaderFacts({ p }: { p: CompanyProfile }) {
  const facts: React.ReactNode[] = [];
  const latestCapex = p.capex?.quarters.at(-1) ?? null;
  const lastYear = p.capex?.years.at(-1) ?? null;
  const usd = p.equityRaised.totals;
  const roundFact = p.latestRound ? (
    <Fact
      key="round"
      label="Last round"
      value={<Link href={p.latestRound.href}>{p.latestRound.event.roundLabel ?? p.latestRound.typeLabels[0]}</Link>}
      sub={
        <>
          {formatPartialDate(p.latestRound.date)} · {p.latestRound.statusLabel} · <SourceLink source={p.latestRound.sources[0]} />
        </>
      }
    />
  ) : null;
  const valuationFact = p.latestValuation ? (
    <Fact
      key="valuation"
      label="Post-money valuation"
      value={<Amount view={p.latestValuation} emphasis />}
      sub={
        <>
          {formatPartialDate(p.latestValuation.amount.asOf)} · {p.latestValuation.statusLabel} · <SourceLink source={p.latestValuation.source} />
        </>
      }
    />
  ) : null;
  const equityFact =
    usd.length > 0 ? (
      <Fact
        key="equity"
        label="Equity raised*"
        value={<span className={styles.num}>{usd.map((t) => formatTotal(t)).join(" + ")}</span>}
        sub={
          <>
            in {p.equityRaised.rounds} closed {p.equityRaised.rounds === 1 ? "round" : "rounds"} Tokscale tracks ·{" "}
            <Link href={methodologyPath()}>method</Link>
          </>
        }
      />
    ) : null;
  const runRateFact = p.latestRunRate ? (
    <Fact
      key="runrate"
      label="Run-rate revenue"
      value={<span className={styles.num}>{p.latestRunRate.short}</span>}
      sub={
        <>
          {formatPartialDate(p.latestRunRate.asOf)} · company stated · <SourceLink source={p.latestRunRate.source} />
        </>
      }
    />
  ) : null;
  const computeFact =
    p.compute.length > 0 ? (
      <Fact
        key="compute"
        label="Compute deals"
        value={<a href="#compute">{p.compute.length}</a>}
        sub="Dollar values and capacity below"
      />
    ) : null;
  const capexFact = latestCapex ? (
    <Fact
      key="capex"
      label={`Cash capex, ${fiscalLabel(latestCapex)}`}
      value={<span className={styles.num}>{formatMoney(latestCapex.value, latestCapex.currency)}</span>}
      sub={
        <>
          {calendarSpanLabel(latestCapex.start, latestCapex.end)}
          {latestCapex.derived ? " · derived" : ""} · <CapexSources sources={latestCapex.sources} compact />
        </>
      }
    />
  ) : null;
  const yearFact = lastYear ? (
    <Fact
      key="fy"
      label={`Cash capex, FY${lastYear.fiscalYear}`}
      value={<span className={styles.num}>{formatMoney(lastYear.value, lastYear.currency)}</span>}
      sub={
        <>
          {calendarSpanLabel(lastYear.start, lastYear.end)} · <a href={lastYear.source.url}>{lastYear.source.form}</a>
        </>
      }
    />
  ) : null;
  const allCapacity = p.compute.flatMap((e) => e.quantities);
  const mwCapacity = allCapacity.filter((q) => q.quantity.measure === "it_load_mw");
  const capacity = (mwCapacity.length > 0 ? mwCapacity : allCapacity.filter((q) => q.quantity.measure === "capacity_gw")).sort(
    (a, b) => b.quantity.value - a.quantity.value
  );
  const capacityFact =
    capacity.length > 0 ? (
      <Fact
        key="capacity"
        label={`Largest stated ${QUANTITY_UNITS[capacity[0].quantity.measure].label}`}
        value={<span className={styles.num}>{capacity[0].short}</span>}
        sub={`${capacity.length} ${capacity.length === 1 ? "deal states" : "deals state"} this measure; they are not added`}
      />
    ) : null;
  const tickerFact = p.org.ticker ? <Fact key="ticker" label="Listed" value={<span className={styles.num}>{p.org.org.ids.tickers[0].symbol}</span>} sub={p.org.ticker} /> : null;
  // Largest stated contract where this company is the provider or landlord (USD only, so values compare).
  const providerDeals = p.compute.filter((e) =>
    e.participants.some((x) => x.org.id === p.org.id && x.roles.some((r) => r === "provider" || r === "lessor"))
  );
  const contract =
    providerDeals
      .flatMap((e) =>
        e.amounts
          .filter(
            (a) =>
              a.skipped === null &&
              (a.amount.measure === "contract_value" || a.amount.measure === "compute_commitment") &&
              a.amount.value !== null &&
              a.amount.currency === "USD" &&
              a.amount.status !== "reported" &&
              a.amount.status !== "estimated"
          )
          .map((a) => ({ a, e }))
      )
      .sort((x, y) => (y.a.amount.value ?? 0) - (x.a.amount.value ?? 0))[0] ?? null;
  const contractFact = contract ? (
    <Fact
      key="contract"
      label="Largest customer contract"
      value={<Amount view={contract.a} emphasis />}
      sub={
        <>
          {contract.e.participants
            .filter((x) => x.roles.some((r) => r === "buyer" || r === "lessee"))
            .map((x) => x.org.displayName)
            .join(", ")}{" "}
          · {formatPartialDate(contract.e.date)} · <SourceLink source={contract.a.source} />
        </>
      }
    />
  ) : null;
  const lastFinancing = p.rounds[0] ?? null;
  const lastFinancingAmount = lastFinancing ? financingAmount(lastFinancing, undefined, p.org.id) : null;
  const lastFinancingFact =
    lastFinancing && !p.latestRound ? (
      <Fact
        key="financing"
        label="Last financing"
        value={
          lastFinancingAmount ? (
            <Amount view={lastFinancingAmount} />
          ) : (
            <Link href={lastFinancing.href}>{financingLabel(lastFinancing)}</Link>
          )
        }
        sub={
          <>
            <Link href={lastFinancing.href}>{financingLabel(lastFinancing)}</Link> ·{" "}
            {formatPartialDate(lastFinancing.date)} · {lastFinancingAmount?.statusLabel ?? lastFinancing.statusLabel} · <SourceLink source={lastFinancingAmount?.source ?? lastFinancing.sources[0]} />
          </>
        }
      />
    ) : null;
  const parentFact = p.parent ? (
    <Fact
      key="parent"
      label="Parent"
      value={p.parent.org.companyHref ? <Link href={p.parent.org.companyHref}>{p.parent.org.name}</Link> : p.parent.org.name}
      sub={
        <>
          since {formatPartialDate(p.parent.since)} · <SourceLink source={p.parent.source} />
        </>
      }
    />
  ) : null;

  switch (p.template) {
    case "lab":
    case "app":
      facts.push(parentFact, roundFact, valuationFact, equityFact, runRateFact, computeFact, capexFact);
      break;
    case "neocloud":
    case "cloud":
      facts.push(tickerFact, capexFact ?? yearFact, contractFact, roundFact ?? lastFinancingFact, equityFact, computeFact, capacityFact);
      break;
    case "hyperscaler":
      facts.push(tickerFact, capexFact, yearFact, contractFact, computeFact);
      break;
    case "chips":
      facts.push(tickerFact, capexFact, yearFact, contractFact, lastFinancingFact, computeFact);
      break;
    case "power":
      facts.push(tickerFact, capacityFact, contractFact, computeFact, capexFact ?? yearFact, lastFinancingFact);
      break;
  }
  const shown = facts.filter(Boolean).slice(0, 5);
  if (shown.length === 0) {
    return (
      <p className={styles.callout}>
        Tokscale has no sourced number for {p.org.name} yet. Events and filings are added after an editor checks the primary
        source.
      </p>
    );
  }
  return (
    <dl className={styles.stats} style={{ "--cols": shown.length } as React.CSSProperties} aria-label={`${p.org.name} key facts`}>
      {shown}
    </dl>
  );
}

function FundingTable({ p }: { p: CompanyProfile }) {
  const scale = maxByCurrency(
    p.rounds.map((e) => {
      const main = financingAmount(e, [...EQUITY_MEASURES, "ipo_proceeds", "form_d_amount_sold"], p.org.id) ?? financingAmount(e, ["debt_raised"], p.org.id);
      return { value: main?.amount.value ?? null, currency: main?.amount.currency ?? "" };
    })
  );
  return (
    <table className={`${styles.ledgerTable} ${styles.ledgerStack}`}>
      <thead>
        <tr>
          <th scope="col">Date</th>
          <th scope="col">Round</th>
          <th scope="col" className={`${styles.right} ${styles.amountCell}`}>
            Equity
          </th>
          <th scope="col" className={`${styles.right} ${styles.amountCell}`}>
            Debt
          </th>
          <th scope="col" className={styles.right}>
            Post-money
          </th>
          <th scope="col">Leads</th>
          <th scope="col">Status</th>
        </tr>
      </thead>
      <tbody>
        {p.rounds.map((e) => {
          const equity = financingAmount(e, [...EQUITY_MEASURES, "ipo_proceeds", "form_d_amount_sold"], p.org.id) ?? undefined;
          const debt = financingAmount(e, ["debt_raised"], p.org.id) ?? undefined;
          const leads = e.participants.filter((x) => x.roles.includes("lead_investor"));
          const coLeads = e.participants.filter((x) => x.roles.includes("co_lead") && !x.roles.includes("lead_investor"));
          const includes = e.relations.filter((r) => r.type === "includes");
          return (
            <FundingRow
              key={e.id}
              e={e}
              equity={equity}
              debt={debt}
              leads={leads.map((l) => l.org)}
              coLeads={coLeads.map((l) => l.org)}
              includes={includes}
              scale={scale}
            />
          );
        })}
      </tbody>
    </table>
  );
}

function FundingRow({
  e,
  equity,
  debt,
  leads,
  coLeads,
  includes,
  scale,
}: {
  e: EventView;
  equity: EventView["amounts"][number] | undefined;
  debt: EventView["amounts"][number] | undefined;
  leads: OrgView[];
  coLeads: OrgView[];
  includes: EventView["relations"];
  scale: Map<string, number>;
}) {
  // Rounds have a label ("Series H"); other financings show the instrument the source names.
  const detail =
    equity && equity.amount.measure === "form_d_amount_sold"
      ? "Amount sold under Form D"
      : e.event.roundLabel
        ? null
        : (debt?.amount.label ?? equity?.amount.label ?? null);
  return (
    <>
      <tr data-row-link="">
        <td className={styles.nowrap} data-label="Date">
          {formatPartialDate(e.date)}
        </td>
        <td className={styles.full}>
          <Link className={styles.cellMain} href={e.href} data-row-primary="">
            {financingLabel(e)}
          </Link>
          {detail ? <span className={styles.cellSub}>{detail}</span> : null}
          {equity?.skipped === "included_in" || debt?.skipped === "included_in" ? <span className={styles.cellSub}>counted in a later event</span> : null}
        </td>
        <td className={`${styles.right} ${styles.amountCell} ${equity ? "" : styles.emptyStacked}`} data-label="Equity">
          {equity ? (
            <>
              <Amount view={equity} />
              <AmountBar value={equity.amount.value} max={scale.get(equity.amount.currency) ?? 0} />
            </>
          ) : (
            <Missing />
          )}
        </td>
        <td className={`${styles.right} ${styles.amountCell} ${debt ? "" : styles.emptyStacked}`} data-label="Debt">
          {debt ? (
            <>
              <Amount view={debt} />
              <AmountBar value={debt.amount.value} max={scale.get(debt.amount.currency) ?? 0} />
            </>
          ) : (
            <Missing />
          )}
        </td>
        <td className={`${styles.right} ${e.valuation ? "" : styles.emptyStacked}`} data-label="Post-money">
          {e.valuation ? <Amount view={e.valuation} /> : <Missing />}
        </td>
        <td className={leads.length + coLeads.length > 0 ? "" : styles.emptyStacked} data-label="Leads">
          {leads.length > 0 ? leads.map((o) => o.displayName).join(", ") : coLeads.length > 0 ? null : <Missing reason="No lead named in the source" />}
          {coLeads.length > 0 ? (
            <span className={leads.length > 0 ? styles.cellSub : undefined}>
              {leads.length > 0 ? "Co-led: " : "Co-led by "}
              {coLeads.map((o) => o.displayName).join(", ")}
            </span>
          ) : null}
        </td>
        <td data-label="Status">
          <span className={styles.statusText}>{equity && debt ? e.statusLabel : (equity ?? debt)?.statusLabel ?? e.statusLabel}</span> <SourceLink source={(equity ?? debt)?.source ?? e.sources[0]} />
        </td>
      </tr>
      {includes.map((r) => (
        <tr className={styles.subRow} key={r.event.id}>
          <td />
          <td colSpan={6}>
            └ includes <Link href={r.event.href}>{r.event.title}</Link> ({formatPartialDate(r.event.date)}), counted once
          </td>
        </tr>
      ))}
    </>
  );
}

function ComputeTable({ p }: { p: CompanyProfile }) {
  return (
    <table className={`${styles.ledgerTable} ${styles.ledgerStack}`}>
      <thead>
        <tr>
          <th scope="col">Date</th>
          <th scope="col">Deal</th>
          <th scope="col">Role</th>
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
        {p.compute.map((e) => {
          const role = e.participants.find((x) => x.org.id === p.org.id)?.roles.map((r) => ROLE_LABELS[r]).join(", ") ?? "";
          const value = e.amounts.find(
            (a) => (a.amount.measure === "contract_value" || a.amount.measure === "compute_commitment" || a.amount.measure === "purchase_price") && a.skipped === null
          );
          const alternative = e.amounts.find((a) => a.skipped === "alternative");
          const capacity = e.quantities.filter((q) => q.quantity.measure !== "shares" && q.quantity.measure !== "investors");
          const others = e.participants.filter((x) => x.org.id !== p.org.id).map((x) => x.org.displayName);
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
                  with {others.join(", ")} · {e.typeLabels.join(", ")}
                </span>
              </td>
              <td data-label="Role">{role}</td>
              <td className={`${styles.right} ${styles.amountCell} ${value ? "" : styles.emptyStacked}`} data-label="Value">
                {value ? (
                  <>
                    <Amount view={value} />
                    {value.term ? <span className={styles.cellSub}>{value.term}</span> : null}
                    {alternative ? <span className={styles.cellSub}>{alternative.short} with options</span> : null}
                  </>
                ) : (
                  <Missing reason="No dollar value stated in the source" />
                )}
              </td>
              <td className={`${styles.right} ${styles.num} ${capacity.length > 0 ? "" : styles.emptyStacked}`} data-label="Capacity">
                {capacity.length > 0 ? capacity.map((q) => q.short).join(", ") : <Missing reason="No capacity stated in the source" />}
              </td>
              <td data-label="Source">
                <SourceLink source={value?.source ?? capacity[0]?.source ?? e.sources[0]} />
              </td>
            </tr>
          );
        })}
      </tbody>
    </table>
  );
}

function ModelsSection({ usage, org }: { usage: LabUsage | null | "unavailable"; org: OrgView }) {
  if (usage === "unavailable") {
    return <p className={styles.empty}>Tokscale usage data is not available right now.</p>;
  }
  if (!usage || usage.models.length === 0) {
    return <p className={styles.empty}>No {org.name} models in Tokscale submissions in the last 30 days.</p>;
  }
  const top = usage.models.slice(0, 8);
  return (
    <>
      <table className={`${styles.ledgerTable} ${styles.ledgerStack}`}>
        <caption>
          Among Tokscale users, last 30 days ({formatPartialDate(usage.coverage.start)} to {formatPartialDate(usage.coverage.end)}).
          Tokscale usage is not market share.
        </caption>
        <thead>
          <tr>
            <th scope="col">Model</th>
            <th scope="col" className={styles.right}>
              Active users
            </th>
            <th scope="col" className={styles.right}>
              Adoption
            </th>
            <th scope="col" className={styles.right}>
              Tokens
            </th>
          </tr>
        </thead>
        <tbody>
          {top.map((m) => (
            <tr key={m.key} data-row-link="">
              <td className={styles.full}>
                <span className={styles.modelCell}>
                  <ModelMark model={m} size={24} />
                  <span>
                    <Link className={styles.cellMain} href={`/models/${encodeURIComponent(m.key)}`} data-row-primary="">
                      {m.name}
                    </Link>
                    <span className={styles.cellSub}>{m.key}</span>
                  </span>
                </span>
              </td>
              <td className={`${styles.right} ${styles.num}`} data-label="Active users">
                {m.activeUsers.toLocaleString("en-US")}
              </td>
              <td className={`${styles.right} ${styles.num}`} data-label="Adoption">
                {(m.adoption * 100).toFixed(0)}%
              </td>
              <td className={`${styles.right} ${styles.num}`} data-label="Tokens">
                {compactCount(m.tokens)}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
      <p className={styles.note}>
        {usage.activeModels} {org.name} {usage.activeModels === 1 ? "model" : "models"} used ·{" "}
        {usage.share !== null ? `${(usage.share * 100).toFixed(1)}% of tokens from identified models` : "no tokens"} among{" "}
        {usage.coverage.accounts.toLocaleString("en-US")} Tokscale accounts
        {usage.coverage.lastDataAt ? ` · data through ${formatPartialDate(usage.coverage.lastDataAt.slice(0, 10))}` : ""}.{" "}
        <Link className={styles.link} href="/models">
          All models →
        </Link>
      </p>
    </>
  );
}

function compactCount(value: number): string {
  const units: Array<[number, string]> = [
    [1e12, "T"],
    [1e9, "B"],
    [1e6, "M"],
    [1e3, "K"],
  ];
  for (const [size, suffix] of units) if (value >= size) return `${(value / size).toFixed(1)}${suffix}`;
  return value.toLocaleString("en-US");
}

function orgJsonLd(p: CompanyProfile) {
  const org = p.org.org;
  const sameAs = [org.website, org.ids.wikidata ? `https://www.wikidata.org/wiki/${org.ids.wikidata}` : null].filter(Boolean);
  return {
    "@context": "https://schema.org",
    "@graph": [
      {
        "@type": org.status === "public" ? "Corporation" : "Organization",
        name: org.name,
        ...(org.legalNames[0] ? { legalName: org.legalNames[0] } : {}),
        ...(org.website ? { url: org.website } : {}),
        ...(org.founded ? { foundingDate: org.founded.date } : {}),
        ...(sameAs.length > 0 ? { sameAs } : {}),
        ...(org.ids.tickers[0] ? { tickerSymbol: `${org.ids.tickers[0].exchange}: ${org.ids.tickers[0].symbol}` } : {}),
        ...(org.ids.secCik[0]
          ? { identifier: { "@type": "PropertyValue", propertyID: "SEC CIK", value: String(Number(org.ids.secCik[0])) } }
          : {}),
        ...(p.parent ? { parentOrganization: { "@type": "Organization", name: p.parent.org.name, ...(p.parent.org.companyHref ? { url: absoluteUrl(p.parent.org.companyHref) } : {}) } } : {}),
        description: org.description,
        mainEntityOfPage: companyUrl(org.slug),
      },
      {
        "@type": "BreadcrumbList",
        itemListElement: [
          { "@type": "ListItem", position: 1, name: "Capital", item: absoluteUrl(capitalPath()) },
          { "@type": "ListItem", position: 2, name: "Companies", item: absoluteUrl(companiesPath()) },
          { "@type": "ListItem", position: 3, name: org.name, item: companyUrl(org.slug) },
        ],
      },
    ],
  };
}

async function V2Profile({ p }: { p: CompanyProfile }) {
  const org = p.org.org;
  const labSlugs = org.labSlugs;
  let usage: LabUsage | null | "unavailable" = null;
  if (labSlugs.length > 0) {
    const snapshot = await loadUsageSnapshot();
    usage = snapshot ? usageForLabs(snapshot, labSlugs) : "unavailable";
  }
  const sections = [
    p.rounds.length > 0 || p.template === "lab" ? ["funding", "Funding"] : null,
    p.investors.length > 0 ? ["investors", "Investors"] : null,
    p.compute.length > 0 ? ["compute", "Compute"] : null,
    labSlugs.length > 0 ? ["models", "Models on Tokscale"] : null,
    p.capex ? ["spending", "Spending"] : null,
    p.statements.length > 0 ? ["metrics", "Metrics"] : null,
    ["timeline", "Timeline"],
    ["sources", "Sources"],
  ].filter((s): s is string[] => s !== null);
  const leads = p.investors.filter((i) => i.roles.includes("lead_investor"));
  const coLeads = p.investors.filter((i) => i.roles.includes("co_lead") && !i.roles.includes("lead_investor"));
  const strategic = p.investors.filter((i) => i.roles.includes("strategic_investor") && !i.roles.includes("lead_investor") && !i.roles.includes("co_lead"));
  const others = p.investors.filter((i) => !leads.includes(i) && !coLeads.includes(i) && !strategic.includes(i));
  const eyebrow = [
    org.categories.map((c) => CATEGORY_LABELS[c]).join(", "),
    p.org.statusText,
    org.hq ? [org.hq.city, org.hq.country].filter(Boolean).join(", ") : null,
  ]
    .filter(Boolean)
    .join(" · ");

  return (
    <>
      <JsonLd data={orgJsonLd(p)} />
      <Breadcrumbs items={[{ label: "Capital", href: capitalPath() }, { label: "Companies", href: companiesPath() }, { label: p.org.name }]} />
      <header className={styles.profileHead}>
        <OrgMark org={p.org} large />
        <div>
          <h1 className={`${styles.display} ${styles.profileTitle}`}>{p.org.name}</h1>
          <p className={styles.lede}>
            {titleCase(eyebrow)}. {org.description}
          </p>
          <p className={styles.metaLine}>
            {org.legalNames[0] ? <span>Legal name: {org.legalNames[0]}</span> : null}
            {p.founded ? (
              <span>
                Founded {formatPartialDate(p.founded.date)} (
                <a className={styles.link} href={p.founded.source.url} target="_blank" rel="noopener noreferrer">
                  {p.founded.source.publisher}
                </a>
                )
              </span>
            ) : null}
            {p.parent ? (
              <span>
                Part of{" "}
                {p.parent.org.companyHref ? (
                  <Link className={styles.link} href={p.parent.org.companyHref}>
                    {p.parent.org.name}
                  </Link>
                ) : (
                  p.parent.org.name
                )}{" "}
                since {formatPartialDate(p.parent.since)}
              </span>
            ) : null}
            <span>Reviewed {formatPartialDate(p.reviewedAt)}</span>
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
            {org.ids.githubOrg ? (
              <li>
                <a href={`https://github.com/${org.ids.githubOrg}`} target="_blank" rel="noopener noreferrer">
                  github.com/{org.ids.githubOrg}
                </a>
              </li>
            ) : null}
            {org.ids.hfOrg ? (
              <li>
                <a href={`https://huggingface.co/${org.ids.hfOrg}`} target="_blank" rel="noopener noreferrer">
                  huggingface.co/{org.ids.hfOrg}
                </a>
              </li>
            ) : null}
            {org.ids.secCik[0] ? (
              <li>
                <a href={edgarFilingsUrl(org.ids.secCik[0])} target="_blank" rel="noopener noreferrer">
                  SEC CIK {Number(org.ids.secCik[0])}
                </a>
              </li>
            ) : null}
            {p.org.investorHref ? (
              <li>
                <Link href={p.org.investorHref}>{p.org.name} as an investor</Link>
              </li>
            ) : null}
          </ul>
        </div>
      </header>

      <HeaderFacts p={p} />

      <nav aria-label="On this page">
        <ul className={styles.sectionNav}>
          {sections.map(([id, label]) => (
            <li key={id}>
              <a href={`#${id}`}>{label}</a>
            </li>
          ))}
        </ul>
      </nav>

      {sections.some(([id]) => id === "funding") ? (
        <section className={styles.section} id="funding" aria-labelledby="funding-title" style={{ borderTop: 0 }}>
          <div className={styles.sectionHead}>
            <h2 id="funding-title" className={`${styles.display} ${styles.sectionTitle}`}>
              Funding
            </h2>
            <p className={styles.sectionNote}>Financing events where {p.org.name} raised money or filed for it</p>
          </div>
          {p.rounds.length > 0 ? (
            <FundingTable p={p} />
          ) : (
            <p className={styles.empty}>No financing event with a primary source is tracked yet.</p>
          )}
          <p className={styles.note}>
            * Equity raised counts closed primary equity rounds only, per currency. It excludes debt, secondary sales, Form D
            amounts and money counted inside a later round. Other trackers often add some of these, so their totals differ.{" "}
            <Link className={styles.link} href={methodologyPath()}>
              Method →
            </Link>
          </p>
        </section>
      ) : null}

      {p.investors.length > 0 ? (
        <section className={styles.section} id="investors" aria-labelledby="investors-title">
          <div className={styles.sectionHead}>
            <h2 id="investors-title" className={`${styles.display} ${styles.sectionTitle}`}>
              Investors named in primary sources
            </h2>
          </div>
          <dl className={styles.dl}>
            {(
              [
                ["Led a round", leads],
                ["Co-led", coLeads],
                ["Strategic", strategic],
                ["Participated", others],
              ] as const
            )
              .filter(([, list]) => list.length > 0)
              .map(([label, list]) => (
                <div key={label} style={{ display: "contents" }}>
                  <dt>{label}</dt>
                  <dd className={styles.chipsInline}>
                    {list.map((i) =>
                      i.org.href ? (
                        <Link key={i.org.id} href={i.org.investorHref ?? i.org.href}>
                          {i.org.name}
                        </Link>
                      ) : (
                        <span key={i.org.id}>{i.org.name}</span>
                      )
                    )}
                  </dd>
                </div>
              ))}
          </dl>
          <p className={styles.note}>Amounts per investor are shown only when a source states them. Other named investors are listed on each round page.</p>
        </section>
      ) : null}

      {p.compute.length > 0 ? (
        <section className={styles.section} id="compute" aria-labelledby="compute-title">
          <div className={styles.sectionHead}>
            <h2 id="compute-title" className={`${styles.display} ${styles.sectionTitle}`}>
              Compute and infrastructure
            </h2>
            <p className={styles.sectionNote}>Stated contract totals over multi-year terms, not annual spend</p>
          </div>
          <ComputeTable p={p} />
        </section>
      ) : null}

      {labSlugs.length > 0 ? (
        <section className={styles.section} id="models" aria-labelledby="models-title">
          <div className={styles.sectionHead}>
            <h2 id="models-title" className={`${styles.display} ${styles.sectionTitle}`}>
              Models and usage on Tokscale
            </h2>
            <p className={styles.sectionNote}>Among Tokscale users only</p>
          </div>
          <ModelsSection usage={usage} org={p.org} />
        </section>
      ) : null}

      {p.capex ? (
        <section className={styles.section} id="spending" aria-labelledby="spending-title">
          <div className={styles.sectionHead}>
            <h2 id="spending-title" className={`${styles.display} ${styles.sectionTitle}`}>
              Cash paid for property and equipment
            </h2>
            <p className={styles.sectionNote}>SEC XBRL, by fiscal quarter · hollow bars are derived</p>
          </div>
          <CapexTable view={p.capex} name={p.org.name} />
        </section>
      ) : null}

      {p.statements.length > 0 ? (
        <section className={styles.section} id="metrics" aria-labelledby="metrics-title">
          <div className={styles.sectionHead}>
            <h2 id="metrics-title" className={`${styles.display} ${styles.sectionTitle}`}>
              Metrics
            </h2>
            <p className={styles.sectionNote}>Numbers the company states or files about itself</p>
          </div>
          <table className={`${styles.ledgerTable} ${styles.ledgerStack}`}>
            <thead>
              <tr>
                <th scope="col">Metric</th>
                <th scope="col" className={styles.right}>
                  Value
                </th>
                <th scope="col">As of</th>
                <th scope="col">Status</th>
                <th scope="col">Source</th>
              </tr>
            </thead>
            <tbody>
              {p.statements.map((s) => (
                <tr key={s.id}>
                  <td className={styles.full}>
                    <span className={styles.cellMain}>{s.label}</span>
                    {s.period ? <span className={styles.cellSub}>{s.period.label}</span> : null}
                    <blockquote className={styles.quoteBlock}>“{s.quote}”</blockquote>
                  </td>
                  <td className={`${styles.right} ${styles.num}`} data-label="Value">
                    {s.short}
                  </td>
                  <td className={styles.nowrap} data-label="As of">
                    {formatPartialDate(s.asOf)}
                  </td>
                  <td data-label="Status">
                    <span className={`${styles.statusText} ${s.status === "reported" ? styles.reported : ""}`}>{s.statusLabel}</span>
                  </td>
                  <td data-label="Source">
                    <SourceLink source={s.source} />
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </section>
      ) : null}

      {p.otherDeals.length > 0 || p.asInvestor.length > 0 ? (
        <section className={styles.section} aria-labelledby="other-title">
          <div className={styles.sectionHead}>
            <h2 id="other-title" className={`${styles.display} ${styles.sectionTitle}`}>
              {p.asInvestor.length > 0 ? "Deals and investments" : "Other deals"}
            </h2>
          </div>
          <EventFeed events={[...p.otherDeals, ...p.asInvestor].sort((a, b) => b.date.localeCompare(a.date))} />
        </section>
      ) : null}

      <section className={styles.section} id="timeline" aria-labelledby="timeline-title">
        <div className={styles.sectionHead}>
          <h2 id="timeline-title" className={`${styles.display} ${styles.sectionTitle}`}>
            Timeline
          </h2>
          <p className={styles.sectionNote}>Every tracked event, newest first</p>
        </div>
        <EventFeed events={p.timeline} />
      </section>

      {p.subsidiaries.length > 0 || p.related.length > 0 || p.counterparties.length > 0 ? (
        <section className={styles.section} aria-labelledby="related-title">
          <div className={styles.sectionHead}>
            <h2 id="related-title" className={`${styles.display} ${styles.sectionTitle}`}>
              Related
            </h2>
          </div>
          <dl className={styles.dl}>
            {p.subsidiaries.length > 0 ? (
              <>
                <dt>Subsidiaries</dt>
                <dd className={styles.chipsInline}>
                  {p.subsidiaries.map((o) => (
                    <Link key={o.id} href={o.href ?? companiesPath()}>
                      {o.name}
                    </Link>
                  ))}
                </dd>
              </>
            ) : null}
            {p.counterparties.length > 0 ? (
              <>
                <dt>Deal counterparties</dt>
                <dd className={styles.chipsInline}>
                  {p.counterparties.map((o) =>
                    o.href ? (
                      <Link key={o.id} href={o.href}>
                        {o.name}
                      </Link>
                    ) : (
                      <span key={o.id}>{o.name}</span>
                    )
                  )}
                </dd>
              </>
            ) : null}
            {p.related.length > 0 ? (
              <>
                <dt>Same category</dt>
                <dd className={styles.chipsInline}>
                  {p.related.map((o) => (
                    <Link key={o.id} href={o.href ?? companiesPath()}>
                      {o.name}
                    </Link>
                  ))}
                </dd>
              </>
            ) : null}
          </dl>
        </section>
      ) : null}

      <section className={styles.section} id="sources" aria-labelledby="sources-title">
        <div className={styles.sectionHead}>
          <h2 id="sources-title" className={`${styles.display} ${styles.sectionTitle}`}>
            Sources
          </h2>
          <p className={styles.sectionNote}>
            {p.sources.length} sources · reviewed {formatPartialDate(p.reviewedAt)}
          </p>
        </div>
        {p.sources.length > 0 ? (
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
        ) : (
          <p className={styles.empty}>No sources yet.</p>
        )}
        <p className={styles.note}>
          Corrections: none. Found an error?{" "}
          <Link className={styles.link} href="/contact">
            Report it
          </Link>
          .
        </p>
      </section>
    </>
  );
}

export default async function CompanyPage({ params }: PageProps) {
  const { slug } = await params;
  const profile = ledger.getCompany(slug);
  if (!profile) notFound();
  return (
    <CapitalShell section="companies">
      <V2Profile p={profile} />
    </CapitalShell>
  );
}
