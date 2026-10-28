import Link from "next/link";
import {
  capitalPath,
  companyPath,
  computePath,
  investorPath,
  roundsPath,
} from "@/lib/capital";
import {
  MEASURE_LABELS,
  QUANTITY_UNITS,
  formatPartialDate,
  ledger,
  sortForDisplay,
  type AmountView,
  type EventView,
  type QuantityView,
} from "@/lib/capital/ledger";
import { Amount } from "./Amount";
import { Breadcrumbs } from "./Breadcrumbs";
import { OrgCell } from "./OrgMark";
import { SourceLink, sourceShortLabel } from "./SourceLink";
import styles from "./capital.module.css";
import { titleCase } from "@/lib/text/titleCase";

const DATE_LABELS: Record<string, string> = {
  announced: "Announced",
  signed: "Signed",
  closed: "Closed",
  filed: "Filed",
  completed: "Completed",
};

function eventCrumbs(e: EventView) {
  if (e.isCompute) return { label: "Compute", href: computePath() };
  if (e.route === "deals") return { label: "Deals", href: `${roundsPath()}#acquisitions` };
  return { label: "Rounds", href: roundsPath() };
}

/** "Amazon → Anthropic", "to Anthropic" or "from Amazon": only the parties the source names. */
function flowText(a: AmountView): string | null {
  if (a.from && a.to) return `${a.from.displayName} → ${a.to.displayName}`;
  if (a.to) return `to ${a.to.displayName}`;
  if (a.from) return `from ${a.from.displayName}`;
  return null;
}

/** The capacity figure for the header: a capacity measure first, then share or investor counts. */
function headlineQuantity(quantities: QuantityView[]): QuantityView | null {
  return quantities.find((q) => q.quantity.measure !== "shares" && q.quantity.measure !== "investors") ?? quantities[0] ?? null;
}

const capitalize = (text: string) => text.charAt(0).toUpperCase() + text.slice(1);

function AmountRow({ a }: { a: AmountView }) {
  const flow = flowText(a);
  const skipped =
    a.skipped === "included_in"
      ? "Counted in a later event, not added again"
      : a.skipped === "part_of"
        ? "Part of the amount above, not added again"
        : a.skipped === "alternative"
          ? "Alternative figure, not added"
          : null;
  return (
    <tr>
      <td className={styles.full}>
        <span className={styles.cellMain}>{a.amount.label ?? MEASURE_LABELS[a.amount.measure]}</span>
        <span className={styles.cellSub}>
          {MEASURE_LABELS[a.amount.measure]}
          {flow ? ` · ${flow}` : ""}
          {a.term ? ` · ${a.term}` : ""}
        </span>
        {a.amount.condition ? <span className={styles.cellSub}>Condition: {a.amount.condition}</span> : null}
        {skipped ? <span className={styles.cellSub}>{skipped}</span> : null}
        <blockquote className={styles.quoteBlock}>“{a.amount.quote}”</blockquote>
      </td>
      <td className={styles.right} data-label="Value">
        <Amount view={a} emphasis={a.skipped === null} />
      </td>
      <td data-label="Status">
        <span className={`${styles.statusText} ${a.amount.status === "reported" ? styles.reported : ""}`}>{a.statusLabel}</span>
      </td>
      <td className={styles.tightCol} data-label="Source">
        <SourceLink source={a.source} />
      </td>
    </tr>
  );
}

export function EventDetail({ e }: { e: EventView }) {
  const crumb = eventCrumbs(e);
  const step = ledger.stepUp(e);
  // Keep the event's headline first, then the other typed amounts in their usual order.
  const ordered = sortForDisplay(
    e.amounts.filter((a) => a.skipped === null && a.amount.measure !== "price_per_share"),
    (a) => a.amount.measure
  );
  const headline = e.headline?.amount;
  const main = (headline && ordered.includes(headline) ? [headline, ...ordered.filter((a) => a !== headline)] : ordered).slice(0, 3);
  // One component the source names inside the first amount ("includes $15 billion of previously committed investments").
  const parts = main[0] ? e.amounts.filter((a) => a.amount.partOf === main[0].amount.id) : [];
  const part = parts.length === 1 ? parts[0] : null;
  const quantity = headlineQuantity(e.quantities);
  const showQuantity = quantity !== null && main.length < 3;
  const showPart = part !== null && main.length + (step ? 1 : 0) + (showQuantity ? 1 : 0) < 4;
  const statCount = main.length + (step ? 1 : 0) + (showQuantity ? 1 : 0) + (showPart ? 1 : 0);
  const relatedStatements = ledger.content.statements.filter((s) => e.event.sourceIds.includes(s.sourceId));
  const dates = Object.entries(e.event.dates).filter((entry): entry is [string, string] => Boolean(entry[1]));

  return (
    <>
      <Breadcrumbs items={[{ label: "Capital", href: capitalPath() }, crumb, { label: e.title }]} />
      <header className={styles.pageHead}>
        <h1 className={`${styles.display} ${styles.pageTitle}`}>{e.title}</h1>
        <p className={styles.lede}>
          {titleCase(`${e.tag} · ${e.typeLabels.join(" · ")} · ${e.statusLabel}`)}. {e.event.summary}
        </p>
        <p className={styles.metaLine}>
          {dates.map(([key, value]) => (
            <span key={key}>
              {DATE_LABELS[key]} {formatPartialDate(value)}
            </span>
          ))}
          <span>Reviewed {formatPartialDate(e.event.reviewedAt)}</span>
          <span>Event ID {e.id}</span>
        </p>
        {!e.primary ? (
          <p className={styles.callout} style={{ marginTop: 16 }}>
            This event rests on press reports only. Its numbers are labeled “reported” with the outlet and never enter
            totals.
          </p>
        ) : null}
      </header>

      {statCount > 0 ? (
        <dl className={styles.stats} style={{ "--cols": Math.min(4, statCount) } as React.CSSProperties}>
          {main.map((a) => (
            <div className={styles.stat} key={a.amount.id}>
              <dt className={styles.statLabel}>{a.amount.label ?? MEASURE_LABELS[a.amount.measure]}</dt>
              <dd className={styles.statValue}>
                <Amount view={a} emphasis />
              </dd>
              <dd className={styles.statSub}>
                {a.statusLabel} · {sourceShortLabel(a.source)}
                {a.term ? ` · ${a.term}` : ""}
              </dd>
            </div>
          ))}
          {showPart && part ? (
            <div className={styles.stat}>
              <dt className={styles.statLabel}>{part.amount.label ?? MEASURE_LABELS[part.amount.measure]}</dt>
              <dd className={styles.statValue}>
                <Amount view={part} />
              </dd>
              <dd className={styles.statSub}>
                Part of the {main[0].short} above, counted once · {sourceShortLabel(part.source)}
              </dd>
            </div>
          ) : null}
          {showQuantity && quantity ? (
            <div className={styles.stat}>
              <dt className={styles.statLabel}>{quantity.quantity.label ?? capitalize(QUANTITY_UNITS[quantity.quantity.measure].label)}</dt>
              <dd className={styles.statValue}>
                <span className={styles.num}>{quantity.short}</span>
              </dd>
              <dd className={styles.statSub}>
                {sourceShortLabel(quantity.source)}
                {quantity.term ? ` · ${quantity.term}` : ""}
              </dd>
            </div>
          ) : null}
          {step ? (
            <div className={styles.stat}>
              <dt className={styles.statLabel}>Step-up vs {step.previous.event.roundLabel ?? "previous round"}</dt>
              <dd className={styles.statValue}>{step.short}</dd>
              <dd className={styles.statSub}>Computed by Tokscale: {step.formula}</dd>
            </div>
          ) : null}
        </dl>
      ) : null}

      {e.amounts.length > 0 ? (
        <section className={styles.section} style={{ marginTop: 32 }} aria-labelledby="amounts">
          <div className={styles.sectionHead}>
            <h2 id="amounts" className={`${styles.display} ${styles.sectionTitle}`}>
              Amounts
            </h2>
            <p className={styles.sectionNote}>Each number with its type, status and the sentence it comes from</p>
          </div>
          <table className={`${styles.ledgerTable} ${styles.ledgerStack} ${styles.narrowTable}`}>
            <thead>
              <tr>
                <th scope="col">Amount</th>
                <th scope="col" className={styles.right}>
                  Value
                </th>
                <th scope="col">Status</th>
                <th scope="col" className={styles.tightCol}>
                  Source
                </th>
              </tr>
            </thead>
            <tbody>
              {e.amounts.map((a) => (
                <AmountRow a={a} key={a.amount.id} />
              ))}
            </tbody>
          </table>
        </section>
      ) : null}

      {e.quantities.length > 0 ? (
        <section className={styles.section} aria-labelledby="capacity">
          <div className={styles.sectionHead}>
            <h2 id="capacity" className={`${styles.display} ${styles.sectionTitle}`}>
              Capacity and counts
            </h2>
          </div>
          <table className={`${styles.ledgerTable} ${styles.ledgerStack}`}>
            <thead>
              <tr>
                <th scope="col">Measure</th>
                <th scope="col" className={styles.right}>
                  Value
                </th>
                <th scope="col">Source</th>
              </tr>
            </thead>
            <tbody>
              {e.quantities.map((q) => (
                <tr key={q.quantity.id}>
                  <td className={styles.full}>
                    <span className={styles.cellMain}>{q.quantity.label ?? QUANTITY_UNITS[q.quantity.measure].label}</span>
                    {q.term ? <span className={styles.cellSub}>{q.term}</span> : null}
                    {q.quantity.condition ? <span className={styles.cellSub}>Condition: {q.quantity.condition}</span> : null}
                    <blockquote className={styles.quoteBlock}>“{q.quantity.quote}”</blockquote>
                  </td>
                  <td className={`${styles.right} ${styles.num}`} data-label="Value">
                    {q.short}
                  </td>
                  <td className={styles.tightCol} data-label="Source">
                    <SourceLink source={q.source} />
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </section>
      ) : null}

      <section className={styles.section} aria-labelledby="participants">
        <div className={styles.sectionHead}>
          <h2 id="participants" className={`${styles.display} ${styles.sectionTitle}`}>
            Participants
          </h2>
          <p className={styles.sectionNote}>
            {e.route === "rounds" ? "Amounts per investor are shown only when a source states them" : "Roles as the source names them"}
          </p>
        </div>
        <table className={`${styles.ledgerTable} ${styles.ledgerStack}`}>
          <thead>
            <tr>
              <th scope="col">Organization</th>
              <th scope="col">Role</th>
              <th scope="col">Type</th>
              <th scope="col">Note</th>
            </tr>
          </thead>
          <tbody>
            {e.participants.map((p) => (
              <tr key={p.org.id} data-row-link={p.org.href ? "" : undefined}>
                <td className={styles.full}>
                  <OrgCell org={p.org} rowLink />
                </td>
                <td data-label="Role">{p.roleLabels.join(", ")}</td>
                <td data-label="Type">{p.org.categoryLabel ?? p.org.investorTypeLabels.join(", ")}</td>
                <td className={p.note ? undefined : styles.emptyStacked} data-label="Note">
                  {p.note ?? <span className={styles.dash}>—</span>}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </section>

      {e.facts.length > 0 ? (
        <section className={styles.section} aria-labelledby="terms">
          <div className={styles.sectionHead}>
            <h2 id="terms" className={`${styles.display} ${styles.sectionTitle}`}>
              Key terms
            </h2>
          </div>
          <dl className={styles.dl} style={{ gridTemplateColumns: "minmax(110px, max-content) minmax(0, 1fr)", gap: "14px 24px" }}>
            {e.facts.map((f) => (
              <div key={f.label} style={{ display: "contents" }}>
                <dt>{f.label}</dt>
                <dd>
                  {f.text} <SourceLink source={f.source} />
                  <blockquote className={styles.quoteBlock}>“{f.quote}”</blockquote>
                </dd>
              </div>
            ))}
          </dl>
        </section>
      ) : null}

      {e.relations.length > 0 || step || relatedStatements.length > 0 ? (
        <section className={styles.section} aria-labelledby="context">
          <div className={styles.sectionHead}>
            <h2 id="context" className={`${styles.display} ${styles.sectionTitle}`}>
              Context
            </h2>
          </div>
          <ul className={styles.rows}>
            {step ? (
              <li className={styles.row} data-row-link="">
                <p className={styles.rowTitle}>
                  Previous round:{" "}
                  <Link href={step.previous.href} data-row-primary="">
                    {step.previous.title}
                  </Link>
                </p>
                <p className={styles.rowMeta}>
                  {formatPartialDate(step.previous.date)}
                  {step.previous.headline ? ` · ${step.previous.headline.short}` : ""}
                  {step.previous.valuation ? ` at ${step.previous.valuation.short} post-money` : ""}
                </p>
              </li>
            ) : null}
            {e.relations.map((r) => (
              <li className={styles.row} key={`${r.type}-${r.event.id}`} data-row-link="">
                <p className={styles.rowTitle}>
                  <Link href={r.event.href} data-row-primary="">
                    {r.event.title}
                  </Link>
                </p>
                <p className={styles.rowMeta}>
                  {formatPartialDate(r.event.date)}
                  {r.note ? ` · ${r.note}` : ""}
                </p>
              </li>
            ))}
            {relatedStatements.map((s) => {
              const view = ledger.statementsOf(s.org).find((x) => x.id === s.id)!;
              return (
                <li className={styles.row} key={s.id}>
                  <p className={styles.rowTitle}>
                    Company metric in the same source: {s.label} {view.short}
                  </p>
                  <p className={styles.rowMeta}>
                    {view.statusLabel}, as of {formatPartialDate(s.asOf)} · “{s.quote}”
                  </p>
                </li>
              );
            })}
          </ul>
        </section>
      ) : null}

      <section className={styles.section} aria-labelledby="sources">
        <div className={styles.sectionHead}>
          <h2 id="sources" className={`${styles.display} ${styles.sectionTitle}`}>
            Sources
          </h2>
        </div>
        <ol className={styles.sourcesOl}>
          {e.sources.map((s) => (
            <li key={s.id}>
              {s.publisher}, “
              <a href={s.url} target="_blank" rel="noopener noreferrer">
                {s.title}
              </a>
              ”{s.publishedAt ? `, ${formatPartialDate(s.publishedAt)}` : ""}
              {s.filing ? ` · Form ${s.filing.form}, accession ${s.filing.accession}` : ""}
              {s.license === "CC-BY-4.0" ? " · CC BY 4.0" : ""} · retrieved {formatPartialDate(s.retrievedAt)}
            </li>
          ))}
        </ol>
        <p className={styles.note}>
          {e.event.corrections.length > 0
            ? e.event.corrections.map((c) => `${formatPartialDate(c.date)}: ${c.text}`).join(" · ")
            : "Corrections: none."}{" "}
          Found an error?{" "}
          <Link className={styles.link} href="/contact">
            Report it
          </Link>
          .
        </p>
      </section>

      <nav className={styles.section} aria-label="Related pages">
        <ul className={styles.chips}>
          {e.participants
            .filter((p) => p.org.href)
            .map((p) => (
              <li key={p.org.id}>
                <Link href={p.org.isCompany ? companyPath(p.org.slug) : investorPath(p.org.slug)}>{p.org.name}</Link>
              </li>
            ))}
        </ul>
      </nav>
    </>
  );
}

/** schema.org data for an event page: a WebPage about the issuer that mentions the participants. No fake "funding round" type. */
export function eventJsonLd(e: EventView, url: string, absolute: (path: string) => string) {
  const org = (o: EventView["participants"][number]["org"]) => ({
    "@type": "Organization",
    name: o.name,
    ...(o.href ? { url: absolute(o.href) } : {}),
  });
  return {
    "@context": "https://schema.org",
    "@graph": [
      {
        "@type": "WebPage",
        name: e.title,
        description: e.event.summary,
        url,
        datePublished: e.date,
        dateModified: [e.event.reviewedAt, ...e.event.corrections.map((c) => c.date)].sort().at(-1),
        ...(e.issuer ? { about: org(e.issuer) } : {}),
        mentions: e.participants.filter((p) => p.org.id !== e.issuer?.id).map((p) => org(p.org)),
        isBasedOn: e.sources.map((s) => s.url),
        image: absolute(e.ogImage),
      },
      {
        "@type": "BreadcrumbList",
        itemListElement: [
          { "@type": "ListItem", position: 1, name: "Capital", item: absolute(capitalPath()) },
          { "@type": "ListItem", position: 2, name: eventCrumbs(e).label, item: absolute(eventCrumbs(e).href.split("#")[0]) },
          { "@type": "ListItem", position: 3, name: e.title, item: url },
        ],
      },
    ],
  };
}

/** "Anthropic Series H: $65B at $965B Post-Money (May 2026)" when sourced; the plain title otherwise. */
export function eventSeoTitle(e: EventView): string {
  const month = new Intl.DateTimeFormat("en-US", { month: "short", year: "numeric", timeZone: "UTC" }).format(
    new Date(`${e.date}T00:00:00Z`)
  );
  const headline = e.primary && e.headline?.kind === "amount" ? e.headline.short : null;
  const valuation = e.primary && e.valuation && e.valuation.amount.status !== "reported" ? e.valuation.short : null;
  if (headline && valuation) return `${e.title}: ${headline} at ${valuation} Post-Money (${month})`;
  if (headline) return `${e.title}: ${headline} (${month})`;
  return `${e.title} (${month})`;
}
