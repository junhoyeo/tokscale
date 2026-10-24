import type { Metadata } from "next";
import Link from "next/link";
import { TOKSCALE_X_HANDLE } from "@/lib/community";
import { capitalPath, methodologyUrl, spendingPath } from "@/lib/capital";
import { AMOUNT_STATUS_LABELS, BASIS_TEXT, MEASURES, MEASURE_LABELS, formatPartialDate, ledger, type Measure } from "@/lib/capital/ledger";
import { SUMMABLE_MEASURES } from "@/lib/capital/ledger/schema";
import { Breadcrumbs } from "../_components/Breadcrumbs";
import { CapitalShell } from "../_components/CapitalShell";
import { JsonLd } from "../_components/JsonLd";
import styles from "../_components/capital.module.css";
import { Eyebrow } from "@/components/ui/Eyebrow";

const TITLE = "How Tokscale Counts AI Capital: Money Types, Statuses and Sources | Tokscale";
const DESCRIPTION =
  "The rules behind Tokscale's Capital pages: typed money, no mixed totals, money counted once, primary sources with quotes, derived capex quarters and corrections.";

export const metadata: Metadata = {
  title: TITLE,
  description: DESCRIPTION,
  alternates: { canonical: methodologyUrl() },
  openGraph: { title: TITLE, description: DESCRIPTION, type: "article", url: methodologyUrl(), siteName: "Tokscale" },
  twitter: { card: "summary", site: TOKSCALE_X_HANDLE, title: TITLE, description: DESCRIPTION },
};

const MEASURE_NOTES: Partial<Record<Measure, string>> = {
  equity_raised: "Size of a primary equity round, as the company states it.",
  debt_raised: "Loans, notes and credit facilities.",
  valuation_post: "A level, not a flow. Never added up.",
  equity_investment: "One investor's stated investment. Often already counted in a round.",
  investment_commitment: "Money promised for later, often with conditions.",
  cumulative_investment: "“Has invested $X to date.” Never added to new money.",
  contract_value: "Stated total over a multi-year term, not annual spend.",
  compute_commitment: "What a lab commits to spend on a cloud or chip supplier.",
  capex_actual: "Cash paid for property and equipment, from SEC XBRL.",
  revenue_run_rate: "Company-stated annualized revenue. Never added up.",
  form_d_amount_sold: "Amount sold under an SEC Form D notice. Not a round size.",
  fund_mark: "A fund's fair-value mark of private shares. Not a valuation.",
  estimated_capital_cost: "An estimate from a dataset. Never in totals.",
};

export default function MethodologyPage() {
  const reviewed = ledger.reviewedAt();
  const sources = [...ledger.content.sources.values()];
  const primary = sources.filter((s) => s.type !== "news" && s.type !== "dataset").length;

  return (
    <CapitalShell section="methodology">
      <JsonLd
        data={{
          "@context": "https://schema.org",
          "@type": "Article",
          headline: "How Tokscale counts AI capital",
          url: methodologyUrl(),
          dateModified: reviewed,
          author: { "@type": "Organization", name: "Tokscale" },
          publisher: { "@type": "Organization", name: "Tokscale" },
        }}
      />
      <Breadcrumbs items={[{ label: "Capital", href: capitalPath() }, { label: "Method" }]} />
      <header className={styles.pageHead}>
        <Eyebrow>Method</Eyebrow>
        <h1 className={`${styles.display} ${styles.pageTitle}`}>How we count</h1>
        <p className={styles.lede}>
          Capital tracks money in the AI economy as typed events. These rules decide what becomes a number on a page, and what
          never does.
        </p>
        <p className={styles.metaLine}>
          <span>{ledger.content.events.size} events</span>
          <span>{sources.length} sources, {primary} of them primary documents</span>
          <span>Reviewed {formatPartialDate(reviewed)}</span>
        </p>
      </header>

      <div className={styles.prose}>
        <h2>One event, one page</h2>
        <p>
          A funding round, a compute contract, an IPO or a filing is one event with one ID and one page. Every company and
          investor in it links to that page, so the same money is never listed twice under different names.
        </p>

        <h2>Every number has a type</h2>
        <p>
          A number is stored with a money type, a currency, a basis, a status, a date and the sentence it comes from. Totals
          exist only inside one money type, one currency and one basis family. Tokscale’s build fails if a page tries to add
          different types together, so “total raised” never mixes equity, debt, commitments and contract values.
        </p>
      </div>

      <section className={styles.section} aria-labelledby="types">
        <div className={styles.sectionHead}>
          <h2 id="types" className={`${styles.display} ${styles.sectionTitle}`}>
            Money types
          </h2>
        </div>
        <table className={`${styles.ledgerTable} ${styles.ledgerStack}`}>
          <thead>
            <tr>
              <th scope="col">Type</th>
              <th scope="col">Can be added up</th>
              <th scope="col">Notes</th>
            </tr>
          </thead>
          <tbody>
            {MEASURES.map((m) => (
              <tr key={m}>
                <td className={styles.full}>
                  <span className={styles.cellMain}>{MEASURE_LABELS[m]}</span>
                </td>
                <td data-label="Can be added up">{SUMMABLE_MEASURES.has(m) ? "Yes, within one currency and basis" : "Never"}</td>
                <td data-label="Notes">{MEASURE_NOTES[m] ?? ""}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </section>

      <section className={styles.section} aria-labelledby="statuses">
        <div className={styles.twoCol}>
          <div>
            <h2 id="statuses" className={`${styles.display} ${styles.sectionTitle}`} style={{ marginBottom: 16 }}>
              Statuses
            </h2>
            <dl className={styles.dl}>
              <dt>{AMOUNT_STATUS_LABELS.announced}</dt>
              <dd>The company said it, in its own announcement.</dd>
              <dt>{AMOUNT_STATUS_LABELS.signed}</dt>
              <dd>A contract is signed, usually disclosed in an SEC filing.</dd>
              <dt>{AMOUNT_STATUS_LABELS.closed}</dt>
              <dd>The company says the money was raised.</dd>
              <dt>{AMOUNT_STATUS_LABELS.filed}</dt>
              <dd>The number is in an SEC filing.</dd>
              <dt>{AMOUNT_STATUS_LABELS.reported}</dt>
              <dd>Only the press reports it. The outlet is named, and the number never enters totals or header facts.</dd>
              <dt>{AMOUNT_STATUS_LABELS.estimated}</dt>
              <dd>An estimate from a dataset. Never in totals.</dd>
              <dt>{AMOUNT_STATUS_LABELS.undisclosed}</dt>
              <dd>Not public. Shown as “Undisclosed”, never as zero.</dd>
            </dl>
            <p className={styles.note}>Rumors and talks are not a status. If the only source says “in talks”, there is no event.</p>
          </div>
          <div>
            <h2 className={`${styles.display} ${styles.sectionTitle}`} style={{ marginBottom: 16 }}>
              Basis
            </h2>
            <dl className={styles.dl}>
              {(["exact", "approximately", "up_to", "up_to_approximately", "more_than", "at_least"] as const).map((b) => (
                <div key={b} style={{ display: "contents" }}>
                  <dt>{BASIS_TEXT[b].symbol || "="}</dt>
                  <dd>{b === "exact" ? "Exact, as stated" : BASIS_TEXT[b].words}</dd>
                </div>
              ))}
            </dl>
            <p className={styles.note}>
              “Up to” values are maximums and “more than” values are minimums. They are added only to values of the same kind.
            </p>
          </div>
        </div>
      </section>

      <section className={styles.section} aria-labelledby="once">
        <div className={styles.prose}>
          <h2 id="once">Money counted once</h2>
          <p>
            Companies often announce an investment and later fold it into a round. Anthropic’s Series H, for example,
            “includes $15 billion of previously committed investments from hyperscalers, including $5 billion from Amazon”.
            Amazon’s $5 billion from April 20, 2026 is linked to the round it is part of, and every total counts it once.
          </p>
          <ul>
            <li>“Included in”: a later event already counts this money.</li>
            <li>“Part of”: a component of a larger number in the same event.</li>
            <li>“Alternative”: another figure for the same thing, such as a contract value with options.</li>
          </ul>
          <h2>Equity raised</h2>
          <p>
            Equity raised is the sum of closed primary equity rounds Tokscale tracks, per currency. It excludes debt, secondary
            sales, Form D amounts and money counted inside a later round. Other trackers use other definitions, so their totals
            differ; some add debt or commitments. Tokscale states its definition next to every total.
          </p>
          <h2>Sources and quotes</h2>
          <p>
            Every number links to a document and carries the sentence it was copied from. Primary sources come first: company
            announcements, SEC filings (8-K, 6-K, S-1, 10-Q, Form D) and exchange filings. Paid databases are not used as
            sources. A script checks that every quote still appears in the cached source document.
          </p>
          <p>
            Epoch AI’s data on AI companies and data centers (CC BY 4.0) is imported into a review inbox with attribution. An
            Epoch row is published only after an editor checks it against a primary source; the published event then cites that
            source. Company identifiers come from Wikidata (CC0), SEC EDGAR, Hugging Face and GitHub. Hong Kong exchange
            filings are not used, because their terms do not allow linking or copying without written consent.
          </p>
          <h2>Cash capex</h2>
          <p>
            The{" "}
            <Link href={spendingPath()}>spending tracker</Link> uses cash paid for property and equipment as tagged in SEC XBRL
            filings. It is not capex guidance and it excludes finance leases. Fiscal labels are kept (Microsoft’s fiscal year
            ends in June, Oracle’s in May). 10-Q cash flows are year to date, so most quarters are derived: the six-month figure
            minus the three-month figure, and so on. Derived quarters are marked. Each value is the first figure the company
            filed for that period. Some companies’ line items also include other assets, such as intangible assets or nuclear
            fuel; the table shows each company’s own line item. Quarters are added across companies only when they cover the
            same calendar quarter in the same currency.
          </p>
          <h2>Tokscale usage</h2>
          <p>
            AI-lab profiles show how Tokscale users use the lab’s models: active users, adoption and tokens in public Tokscale
            submissions. These numbers describe Tokscale users only. They are not market share.
          </p>
          <h2>Coverage</h2>
          <p>
            Capital follows a reviewed list of AI-economy companies and investors, plus organizations named in the events it
            tracks. A “—” means Tokscale has not tracked the number yet. Pages without a sourced fact stay out of search
            results.
          </p>
          <h2>Corrections</h2>
          <p>
            Corrections are listed on the page they change, with the date. Found an error? <Link href="/contact">Contact us</Link>{" "}
            and we will fix it.
          </p>
          <p className={styles.note}>Nothing on these pages is investment advice.</p>
        </div>
      </section>
    </CapitalShell>
  );
}
