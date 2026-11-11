import type { Metadata } from "next";
import Link from "next/link";
import { TOKSCALE_X_HANDLE } from "@/lib/community";
import { absoluteUrl, companiesPath, companiesUrl, methodologyPath } from "@/lib/capital";
import {
  CATEGORIES,
  CATEGORY_LABELS,
  ORG_STATUS_LABELS,
  financingLabel,
  formatMoney,
  formatPartialDate,
  formatTotal,
  ledger,
  type Category,
} from "@/lib/capital/ledger";
import { calendarSpanLabel, fiscalLabel } from "@/lib/capital/ledger/capex";
import {
  DIRECTORY_PRESETS,
  DIRECTORY_VIEWS,
  directoryQuery,
  filterDirectory,
  parseDirectoryParams,
  sortDirectory,
  type DirectoryView,
} from "@/lib/capital/ledger/directory";
import { loadUsageSnapshot, usageForLabs } from "@/lib/capital/ledger/usage";
import { Amount, Missing } from "../_components/Amount";
import { Breadcrumbs } from "../_components/Breadcrumbs";
import { CapitalShell } from "../_components/CapitalShell";
import { DirectoryFilters } from "../_components/DirectoryFilters";
import { JsonLd } from "../_components/JsonLd";
import { CapacityTotal } from "../_components/CapacityTotal";
import { OrgCell } from "../_components/OrgMark";
import styles from "../_components/capital.module.css";
import { Eyebrow } from "@/components/ui/Eyebrow";

const TITLE = "AI Companies: Funding, Valuations, Compute Deals and Capex | Tokscale";
const DESCRIPTION =
  "The companies behind AI: labs, clouds, chips, data centers and power. Funding rounds, valuations, compute contracts and cash capex, each with its source.";

/** Every filtered or sorted view shows the same companies, so the bare URL is canonical. */
export const metadata: Metadata = {
  title: TITLE,
  description: DESCRIPTION,
  alternates: { canonical: companiesUrl() },
  openGraph: { title: TITLE, description: DESCRIPTION, type: "website", url: companiesUrl(), siteName: "Tokscale" },
  twitter: { card: "summary_large_image", site: TOKSCALE_X_HANDLE, title: TITLE, description: DESCRIPTION },
};

const VIEW_LABELS: Record<DirectoryView, string> = {
  overview: "Overview",
  funding: "Funding",
  compute: "Compute",
  public: "Public market",
  models: "Models",
};

const COUNTRY_NAMES: Record<string, string> = {
  US: "United States",
  CN: "China",
  FR: "France",
  GB: "United Kingdom",
  NL: "Netherlands",
  TW: "Taiwan",
  JP: "Japan",
  SG: "Singapore",
  AE: "United Arab Emirates",
};

interface PageProps {
  searchParams: Promise<Record<string, string | string[] | undefined>>;
}

export default async function CompaniesPage({ searchParams }: PageProps) {
  const params = parseDirectoryParams(await searchParams);
  const snapshot = await loadUsageSnapshot();
  const rows = ledger.getDirectoryRows().map((row) => {
    const usage = snapshot ? usageForLabs(snapshot, row.org.org.labSlugs) : null;
    const usd = row.equity.totals.find((t) => t.currency === "USD") ?? null;
    return {
      ...row,
      usage,
      name: row.org.name,
      aliases: row.org.org.aliases,
      legalNames: row.org.org.legalNames,
      tickers: row.org.org.ids.tickers.map((t) => t.symbol),
      status: row.org.org.status,
      valuationUsd: row.valuation && row.valuation.amount.currency === "USD" ? row.valuation.amount.value : null,
      equityUsd: usd?.value ?? null,
      capexUsd: row.capexLatest && row.capexLatest.currency === "USD" ? row.capexLatest.value : null,
      modelsCount: usage ? usage.activeModels : null,
    };
  });
  const shown = sortDirectory(filterDirectory(rows, params), params.sort);
  const countries = [...new Set(rows.map((r) => r.country).filter((c): c is string => Boolean(c)))].sort();
  const query = directoryQuery(params);
  const view = params.view;

  const viewHref = (v: DirectoryView) => `${companiesPath()}${directoryQuery({ ...params, view: v })}`;

  return (
    <CapitalShell section="companies">
      <JsonLd
        data={{
          "@context": "https://schema.org",
          "@type": "ItemList",
          name: "AI-economy companies Tokscale tracks",
          url: companiesUrl(),
          numberOfItems: rows.length,
          itemListElement: rows.map((row, index) => ({
            "@type": "ListItem",
            position: index + 1,
            name: row.org.name,
            url: absoluteUrl(row.org.companyHref ?? companiesPath()),
          })),
        }}
      />
      <Breadcrumbs items={[{ label: "Capital", href: "/capital" }, { label: "Companies" }]} />
      <header className={styles.pageHead}>
        <Eyebrow>Companies</Eyebrow>
        <h1 className={`${styles.display} ${styles.pageTitle}`}>The companies behind AI</h1>
        <p className={styles.lede}>
          AI labs, clouds, chips, data centers and power. {rows.length} companies, each number with its source and date.
        </p>
        <p className={styles.metaLine}>
          <span>Reviewed {formatPartialDate(ledger.reviewedAt())}</span>
        </p>
      </header>

      <DirectoryFilters
        key={query}
        action={companiesPath()}
        q={params.q}
        selects={[
          {
            name: "category",
            label: "Category",
            value: params.category ?? "",
            options: [{ value: "", label: "All categories" }, ...CATEGORIES.map((c: Category) => ({ value: c, label: CATEGORY_LABELS[c] }))],
          },
          {
            name: "country",
            label: "Country",
            value: params.country ?? "",
            options: [{ value: "", label: "All countries" }, ...countries.map((c) => ({ value: c, label: COUNTRY_NAMES[c] ?? c }))],
          },
          {
            name: "status",
            label: "Status",
            value: params.status ?? "",
            options: [
              { value: "", label: "Any status" },
              ...(["private", "public", "subsidiary"] as const).map((s) => ({ value: s, label: ORG_STATUS_LABELS[s] })),
            ],
          },
          {
            name: "sort",
            label: "Sort",
            value: params.sort,
            options: [
              { value: "activity", label: "Latest activity" },
              { value: "name", label: "Name" },
              { value: "valuation", label: "Post-money valuation" },
              { value: "equity", label: "Equity raised" },
              { value: "deals", label: "Compute deals" },
              { value: "capex", label: "Latest quarter capex" },
            ],
          },
        ]}
        checks={[
          { name: "has", value: "rounds", label: "Has rounds", checked: params.has.includes("rounds") },
          { name: "has", value: "compute", label: "Has compute deals", checked: params.has.includes("compute") },
          { name: "has", value: "models", label: "Has models on Tokscale", checked: params.has.includes("models") },
        ]}
        hidden={view === "overview" ? {} : { view }}
        classes={{
          filters: styles.filters,
          field: styles.field,
          input: styles.input,
          select: styles.select,
          check: styles.check,
          button: styles.button,
        }}
      />

      <nav aria-label="Table view">
        <ul className={styles.views}>
          {DIRECTORY_VIEWS.map((v) => (
            <li key={v}>
              <Link href={viewHref(v)} aria-current={v === view ? "true" : undefined} scroll={false}>
                {VIEW_LABELS[v]}
              </Link>
            </li>
          ))}
        </ul>
      </nav>
      <p className={styles.note} style={{ margin: "0 0 12px" }}>
        Views:{" "}
        {DIRECTORY_PRESETS.map((preset, i) => (
          <span key={preset.label}>
            {i > 0 ? " · " : ""}
            <Link className={styles.link} href={`${companiesPath()}${directoryQuery(preset.params)}`} scroll={false}>
              {preset.label}
            </Link>
          </span>
        ))}
        {query ? (
          <>
            {" · "}
            <Link className={styles.link} href={companiesPath()} scroll={false}>
              Reset
            </Link>
          </>
        ) : null}
      </p>

      <section aria-label="Companies">
        <table className={`${styles.ledgerTable} ${styles.ledgerStack}`}>
          <caption>
            {shown.length === rows.length ? `${rows.length} companies` : `${shown.length} of ${rows.length} companies`}
            {view === "models" ? " · Tokscale usage is among Tokscale users in the last 30 days, not market share" : ""}
          </caption>
          <thead>
            <tr>
              <th scope="col">Company</th>
              {view === "overview" ? (
                <>
                  <th scope="col">Category</th>
                  <th scope="col">HQ</th>
                  <th scope="col">Status</th>
                  <th scope="col">Last financing</th>
                  <th scope="col" className={styles.right}>
                    Post-money
                  </th>
                  <th scope="col" className={styles.right}>
                    Equity raised*
                  </th>
                  <th scope="col" className={styles.right}>
                    Compute deals
                  </th>
                </>
              ) : null}
              {view === "funding" ? (
                <>
                  <th scope="col" className={styles.right}>
                    Rounds
                  </th>
                  <th scope="col">Last round</th>
                  <th scope="col" className={styles.right}>
                    Post-money
                  </th>
                  <th scope="col">Leads of last round</th>
                  <th scope="col" className={styles.right}>
                    Equity raised*
                  </th>
                  <th scope="col" className={styles.right}>
                    Form D sold
                  </th>
                </>
              ) : null}
              {view === "compute" ? (
                <>
                  <th scope="col" className={styles.right}>
                    As buyer
                  </th>
                  <th scope="col" className={styles.right}>
                    As provider
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
                </>
              ) : null}
              {view === "public" ? (
                <>
                  <th scope="col">Ticker</th>
                  <th scope="col" className={styles.right}>
                    Latest quarter cash capex
                  </th>
                  <th scope="col" className={styles.right}>
                    Last 4 quarters
                  </th>
                </>
              ) : null}
              {view === "models" ? (
                <>
                  <th scope="col" className={styles.right}>
                    Models used
                  </th>
                  <th scope="col">Top model</th>
                  <th scope="col" className={styles.right}>
                    Share of tokens
                  </th>
                </>
              ) : null}
            </tr>
          </thead>
          <tbody>
            {shown.map((row) => (
              <tr key={row.org.id} data-row-link={row.org.href ? "" : undefined}>
                <td className={styles.full}>
                  <OrgCell
                    org={row.org}
                    rowLink
                    sub={
                      <span className={styles.mobileOnly}>
                        {[row.categories.map((c) => CATEGORY_LABELS[c]).join(", "), row.country, row.org.statusText].filter(Boolean).join(" · ")}
                      </span>
                    }
                  />
                </td>
                {view === "overview" ? (
                  <>
                    <td className={styles.hideMd} data-label="Category">
                      {row.categories.map((c) => CATEGORY_LABELS[c]).join(", ")}
                    </td>
                    <td className={styles.hideMd} data-label="HQ">
                      {row.country ?? <Missing />}
                    </td>
                    <td className={styles.hideMd} data-label="Status">
                      {ORG_STATUS_LABELS[row.org.org.status]}
                      <span className={styles.cellSub}>
                        {row.org.org.status === "subsidiary" || row.org.org.status === "acquired"
                          ? row.org.statusText
                          : (row.org.ticker ?? row.org.org.statusNote ?? "")}
                      </span>
                    </td>
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
                    <td className={`${styles.right} ${row.equity.totals.length > 0 ? "" : styles.emptyStacked}`} data-label="Equity raised*">
                      {row.equity.totals.length > 0 ? (
                        row.equity.totals.map((t) => (
                          <span className={styles.num} key={t.currency} style={{ display: "block" }}>
                            {formatTotal(t)}
                          </span>
                        ))
                      ) : (
                        <Missing />
                      )}
                    </td>
                    <td className={`${styles.right} ${styles.num} ${styles.hideMd}`} data-label="Compute deals">
                      {row.dealsCount > 0 ? row.dealsCount : <Missing />}
                    </td>
                  </>
                ) : null}
                {view === "funding" ? (
                  <>
                    <td className={`${styles.right} ${styles.num} ${row.roundsCount > 0 ? "" : styles.emptyStacked}`} data-label="Rounds">
                      {row.roundsCount > 0 ? row.roundsCount : <Missing />}
                    </td>
                    <td className={row.lastRound ? undefined : styles.emptyStacked} data-label="Last round">
                      {row.lastRound ? (
                        <>
                          <Link href={row.lastRound.href}>{row.lastRound.event.roundLabel ?? row.lastRound.title}</Link>
                          <span className={styles.cellSub}>{formatPartialDate(row.lastRound.date)}</span>
                        </>
                      ) : (
                        <Missing />
                      )}
                    </td>
                    <td className={`${styles.right} ${row.valuation ? "" : styles.emptyStacked}`} data-label="Post-money">
                      {row.valuation ? <Amount view={row.valuation} /> : <Missing />}
                    </td>
                    <td className={row.leadsOfLastRound.length > 0 ? undefined : styles.emptyStacked} data-label="Leads">
                      {row.leadsOfLastRound.length > 0 ? row.leadsOfLastRound.map((o) => o.displayName).join(", ") : <Missing />}
                    </td>
                    <td className={`${styles.right} ${row.equity.totals.length > 0 ? "" : styles.emptyStacked}`} data-label="Equity raised*">
                      {row.equity.totals.length > 0 ? row.equity.totals.map((t) => formatTotal(t)).join(" + ") : <Missing />}
                    </td>
                    <td className={`${styles.right} ${row.formD ? "" : styles.emptyStacked}`} data-label="Form D sold">
                      {row.formD ? <Amount view={row.formD} /> : <Missing />}
                    </td>
                  </>
                ) : null}
                {view === "compute" ? (
                  <>
                    <td className={`${styles.right} ${styles.num} ${row.dealsAsBuyer ? "" : styles.emptyStacked}`} data-label="As buyer">
                      {row.dealsAsBuyer || <Missing />}
                    </td>
                    <td className={`${styles.right} ${styles.num} ${row.dealsAsProvider ? "" : styles.emptyStacked}`} data-label="As provider">
                      {row.dealsAsProvider || <Missing />}
                    </td>
                    <td className={`${styles.right} ${row.largestContract ? "" : styles.emptyStacked}`} data-label="Largest stated value">
                      {row.largestContract ? <Amount view={row.largestContract} /> : <Missing />}
                    </td>
                    <td className={`${styles.right} ${row.capacityMw.ok || row.capacityMw.largest !== null ? "" : styles.emptyStacked}`} data-label="Critical IT load">
                      <CapacityTotal result={row.capacityMw} unit="MW" />
                    </td>
                    <td className={`${styles.right} ${row.capacityGw.ok || row.capacityGw.largest !== null ? "" : styles.emptyStacked}`} data-label="Capacity">
                      <CapacityTotal result={row.capacityGw} unit="GW" />
                    </td>
                  </>
                ) : null}
                {view === "public" ? (
                  <>
                    <td data-label="Ticker">{row.org.ticker ?? <Missing reason="Not listed" />}</td>
                    <td className={styles.right} data-label="Latest quarter">
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
                    <td className={styles.right} data-label="Last 4 quarters">
                      {row.capexTtm ? (
                        <>
                          <span className={styles.num}>{formatMoney(row.capexTtm.value, row.capexTtm.currency)}</span>
                          <span className={styles.cellSub}>computed · {calendarSpanLabel(row.capexTtm.start, row.capexTtm.end)}</span>
                        </>
                      ) : (
                        <Missing />
                      )}
                    </td>
                  </>
                ) : null}
                {view === "models" ? (
                  <>
                    <td className={`${styles.right} ${styles.num}`} data-label="Models used">
                      {row.usage ? row.usage.activeModels : <Missing reason={row.org.org.labSlugs.length > 0 ? "Usage data unavailable" : "No models on Tokscale"} />}
                    </td>
                    <td data-label="Top model">
                      {row.usage && row.usage.models[0] ? (
                        <>
                          <Link href={`/models/${encodeURIComponent(row.usage.models[0].key)}`}>{row.usage.models[0].name}</Link>
                          <span className={styles.cellSub}>{row.usage.models[0].activeUsers} active users</span>
                        </>
                      ) : (
                        <Missing />
                      )}
                    </td>
                    <td className={`${styles.right} ${styles.num}`} data-label="Share of tokens">
                      {row.usage && row.usage.share !== null ? `${(row.usage.share * 100).toFixed(1)}%` : <Missing />}
                    </td>
                  </>
                ) : null}
              </tr>
            ))}
          </tbody>
        </table>
        {shown.length === 0 ? <p className={styles.empty}>No company matches these filters.</p> : null}
        <p className={styles.note}>
          * Equity raised in closed primary rounds Tokscale tracks, per currency. Excludes debt, secondary sales, Form D
          amounts and money already counted inside a later round. “—” means not tracked yet; it is never zero.{" "}
          <Link className={styles.link} href={methodologyPath()}>
            Method →
          </Link>
        </p>
      </section>
    </CapitalShell>
  );
}
