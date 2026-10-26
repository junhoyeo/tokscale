import { quarterLabel } from "../format";
import {
  companyPath,
  dealOgImagePath,
  dealPath,
  investorPath,
  roundOgImagePath,
  roundPath,
  sectorPath,
} from "../urls";
import { deriveCapex, type CapexQuarter, type CapexView } from "./capex";
import { isCompanyFinancing, isFinancing } from "./financing";
import { valuationRatio, type ValuationRatio } from "./ratio";
import { hasPrimarySource, ledgerContent, type LedgerContent } from "./content";
import { formatAmountShort, formatAmountWords, formatMoney, formatQuantity, formatTerm, monogram } from "./format";
import {
  AMOUNT_STATUS_LABELS,
  CATEGORY_LABELS,
  CATEGORY_PLURALS,
  EVENT_STATUS_LABELS,
  EVENT_TYPE_LABELS,
  INVESTOR_TYPE_LABELS,
  ROLE_LABELS,
  feedTag,
} from "./labels";
import type {
  Amount,
  CapitalEvent,
  Category,
  EventType,
  Fact,
  InvestorType,
  Measure,
  Org,
  Quantity,
  Role,
  Source,
  Statement,
} from "./schema";
import { total, totalsByCurrency, tryQuantityTotal, tryTotal, type LedgerTotal } from "./total";

/**
 * Derived views over the Capital v2 ledger. Pages import from "@/lib/capital/ledger".
 *
 * Definitions kept in one place so every page says the same thing:
 * - Equity raised: closed primary equity rounds Tokscale tracks (measure equity_raised, status
 *   closed), per currency. Excludes debt, secondary sales, Form D amounts and money already
 *   counted inside a later round (includedIn) or inside a larger amount (partOf).
 * - Compute deals: deals of a compute, lease, chip or power type. Contract values are stated
 *   totals over multi-year terms, never annual spend.
 * - Cash capex: cash paid for property and equipment from SEC XBRL (see capex.ts).
 * - Press-reported numbers ("reported") are shown with the outlet and never enter totals or
 *   header facts.
 */

export type ProfileTemplate = "lab" | "app" | "hyperscaler" | "cloud" | "neocloud" | "chips" | "power";

const TEMPLATE_BY_CATEGORY: Record<Category, ProfileTemplate> = {
  "ai-lab": "lab",
  "ai-app": "app",
  hyperscaler: "hyperscaler",
  cloud: "cloud",
  neocloud: "neocloud",
  semiconductors: "chips",
  "data-centers": "power",
  power: "power",
};

/** Event types listed on /capital/compute. */
export const COMPUTE_TYPES: ReadonlySet<EventType> = new Set<EventType>([
  "compute_contract",
  "compute_commitment",
  "datacenter_lease",
  "chip_supply",
  "ppa",
  "joint_venture",
  "asset_sale",
]);

const INVESTOR_ROLES: ReadonlySet<Role> = new Set<Role>(["lead_investor", "co_lead", "investor", "strategic_investor"]);
/** Amount statuses that may appear in header facts. Reported (press) and estimated never do. */
const HEADER_STATUSES = new Set(["announced", "signed", "closed", "filed"]);

/** Sector hubs: URL slug -> categories. */
export const SECTORS: ReadonlyArray<{ slug: string; categories: Category[]; name: string; summary: string }> = [
  { slug: "ai-labs", categories: ["ai-lab"], name: "AI labs", summary: "Companies that train frontier and open-weight models." },
  { slug: "neoclouds", categories: ["neocloud"], name: "Neoclouds", summary: "Cloud providers built around GPU capacity for AI." },
  { slug: "hyperscalers", categories: ["hyperscaler"], name: "Hyperscalers", summary: "The largest cloud and platform companies and their capital spending." },
  { slug: "semiconductors", categories: ["semiconductors"], name: "Chips", summary: "Designers and makers of AI accelerators and the chips around them." },
  { slug: "data-centers", categories: ["data-centers"], name: "Data centers", summary: "Developers and landlords of AI data-center campuses." },
  { slug: "power", categories: ["power"], name: "Power", summary: "Generators and developers that supply electricity to data centers." },
];

/* ------------------------------------------------------------------ views */

export interface OrgView {
  org: Org;
  id: string;
  slug: string;
  name: string;
  /** Short form for dense tables. */
  displayName: string;
  isCompany: boolean;
  isInvestor: boolean;
  companyHref: string | null;
  investorHref: string | null;
  /** Company page when the org is a company, else its investor page. */
  href: string | null;
  monogram: string;
  logo: string | null;
  /** Full-bleed square picture, shown when there is no `logo`. */
  avatar: string | null;
  primaryCategory: Category | null;
  categoryLabel: string | null;
  investorTypeLabels: string[];
  /** "NASDAQ: CRWV" */
  ticker: string | null;
  /** "Public · NASDAQ: CRWV", "Private", "Subsidiary of SpaceX". */
  statusText: string;
}

export interface AmountView {
  amount: Amount;
  event: CapitalEvent | null;
  source: Source;
  short: string;
  words: string;
  statusLabel: string;
  term: string | null;
  skipped: "included_in" | "part_of" | "alternative" | null;
  from: OrgView | null;
  to: OrgView | null;
}

export interface QuantityView {
  quantity: Quantity;
  source: Source;
  short: string;
  term: string | null;
}

export interface ParticipantView {
  org: OrgView;
  roles: Role[];
  roleLabels: string[];
  note: string | null;
}

export interface EventRef {
  id: string;
  slug: string;
  title: string;
  date: string;
  href: string;
}

export interface EventView extends EventRef {
  event: CapitalEvent;
  route: "rounds" | "deals";
  ogImage: string;
  tag: string;
  typeLabels: string[];
  statusLabel: string;
  issuer: OrgView | null;
  participants: ParticipantView[];
  amounts: AmountView[];
  quantities: QuantityView[];
  facts: Array<Fact & { source: Source }>;
  sources: Source[];
  primary: boolean;
  /** Indexable when at least one source is primary evidence. */
  indexable: boolean;
  /** The number a feed row shows: a money amount, else a quantity. */
  headline: { short: string; words: string; kind: "amount" | "quantity"; countedElsewhere: boolean; amount: AmountView | null; source: Source } | null;
  /** Second number for rounds: the post-money valuation. */
  valuation: AmountView | null;
  /** "Anthropic ↔ Akamai" for deals; the issuer for financing. */
  partiesText: string;
  relations: Array<{ type: string; note: string | null; event: EventRef }>;
  isCompute: boolean;
}

/* ---------------------------------------------------------------- helpers */

const byDateDesc = (a: { date: string; id: string }, b: { date: string; id: string }) =>
  b.date.localeCompare(a.date) || a.id.localeCompare(b.id);

function eventRoute(event: CapitalEvent): "rounds" | "deals" {
  return event.kind === "deal" || event.kind === "acquisition" ? "deals" : "rounds";
}

function eventHref(event: CapitalEvent): string {
  return eventRoute(event) === "rounds" ? roundPath(event.slug) : dealPath(event.slug);
}

const HEADLINE_MEASURES: Measure[] = [
  "equity_raised",
  "ipo_proceeds",
  "debt_raised",
  "contract_value",
  "compute_commitment",
  "purchase_price",
  "form_d_amount_sold",
  "equity_investment",
  "investment_commitment",
  "form_d_offering_amount",
];

export function createLedgerModel(content: LedgerContent) {
  const { orgs, sources, events, statements } = content;

  const investorParticipations = new Map<string, Array<{ event: CapitalEvent; roles: Role[] }>>();
  for (const event of events.values()) {
    for (const p of event.participants) {
      if (!p.roles.some((r) => INVESTOR_ROLES.has(r) || r === "lender" || r === "acquirer")) continue;
      const list = investorParticipations.get(p.org) ?? [];
      list.push({ event, roles: p.roles });
      investorParticipations.set(p.org, list);
    }
  }

  /** An investor gets a page when it is an investor org (it may still be thin; thin pages are noindex). */
  const hasInvestorPage = (org: Org) => org.kinds.includes("investor");

  const orgViewCache = new Map<string, OrgView>();
  function orgView(id: string): OrgView {
    const cached = orgViewCache.get(id);
    if (cached) return cached;
    const org = orgs.get(id);
    if (!org) throw new Error(`unknown org ${id}`);
    const isCompany = org.kinds.includes("company");
    const isInvestor = org.kinds.includes("investor");
    const companyHref = isCompany ? companyPath(org.slug) : null;
    const investorHref = isInvestor && hasInvestorPage(org) ? investorPath(org.slug) : null;
    const primaryCategory = org.categories[0] ?? null;
    const ticker = org.ids.tickers[0] ? `${org.ids.tickers[0].exchange}: ${org.ids.tickers[0].symbol}` : null;
    const parent = org.parent ? orgs.get(org.parent.org) : undefined;
    const statusText =
      org.status === "public"
        ? ticker
          ? `Public · ${ticker}`
          : `Public${org.statusNote ? ` · ${org.statusNote}` : ""}`
        : org.status === "subsidiary" && parent
          ? `Part of ${parent.name}`
          : org.status === "acquired" && parent
            ? `Acquired by ${parent.name}`
            : org.status === "private"
              ? org.statusNote
                ? `Private · ${org.statusNote}`
                : "Private"
              : org.status.charAt(0).toUpperCase() + org.status.slice(1);
    const view: OrgView = {
      org,
      id: org.id,
      slug: org.slug,
      name: org.name,
      displayName: org.shortName ?? org.name,
      isCompany,
      isInvestor,
      companyHref,
      investorHref,
      href: companyHref ?? investorHref,
      monogram: monogram(org.shortName ?? org.name),
      logo: org.logo ?? null,
      avatar: org.avatar ?? null,
      primaryCategory,
      categoryLabel: primaryCategory ? CATEGORY_LABELS[primaryCategory] : null,
      investorTypeLabels: org.investorTypes.map((t) => INVESTOR_TYPE_LABELS[t]),
      ticker,
      statusText,
    };
    orgViewCache.set(id, view);
    return view;
  }

  const sourceOf = (id: string) => {
    const source = sources.get(id);
    if (!source) throw new Error(`unknown source ${id}`);
    return source;
  };

  function amountView(amount: Amount, event: CapitalEvent | null): AmountView {
    return {
      amount,
      event,
      source: sourceOf(amount.sourceId),
      short: formatAmountShort(amount),
      words: formatAmountWords(amount),
      statusLabel:
        amount.status === "reported" ? `Reported by ${sourceOf(amount.sourceId).publisher}` : AMOUNT_STATUS_LABELS[amount.status],
      term: formatTerm(amount.term),
      skipped: amount.includedIn ? "included_in" : amount.partOf ? "part_of" : amount.alternativeTo ? "alternative" : null,
      from: amount.from ? orgView(amount.from) : null,
      to: amount.to ? orgView(amount.to) : null,
    };
  }

  const refOf = (event: CapitalEvent): EventRef => ({
    id: event.id,
    slug: event.slug,
    title: event.title,
    date: event.date,
    href: eventHref(event),
  });

  const eventViewCache = new Map<string, EventView>();
  function eventView(event: CapitalEvent): EventView {
    const cached = eventViewCache.get(event.id);
    if (cached) return cached;
    const participants = event.participants.map((p) => ({
      org: orgView(p.org),
      roles: p.roles,
      roleLabels: p.roles.map((r) => ROLE_LABELS[r]),
      note: p.note ?? null,
    }));
    const issuerEntry =
      participants.find((p) => p.roles.includes("issuer")) ??
      participants.find((p) => p.roles.includes("borrower") || p.roles.includes("filer")) ??
      participants.find((p) => p.roles.includes("target")) ??
      null;
    const amounts = event.amounts.map((a) => amountView(a, event));
    const quantities = event.quantities.map((q) => ({
      quantity: q,
      source: sourceOf(q.sourceId),
      short: formatQuantity(q),
      term: formatTerm(q.term),
    }));
    const eventSources = event.sourceIds.map(sourceOf);
    const primary = hasPrimarySource(event.sourceIds, sources);

    const isCompute = event.types.some((t) => COMPUTE_TYPES.has(t));
    // A compute deal leads with its contract, not ancillary financing. Financing events still
    // lead with money raised, even when they also include a compute commitment.
    const headlineMeasures: Measure[] =
      event.kind === "deal" && isCompute
        ? ["contract_value", "compute_commitment", ...HEADLINE_MEASURES]
        : event.kind === "acquisition"
          ? ["purchase_price", ...HEADLINE_MEASURES]
          : HEADLINE_MEASURES;
    // A counted amount first; else an amount a later event already counts; else a quantity.
    const headlineAmount =
      headlineMeasures.map((m) => amounts.find((a) => a.amount.measure === m && a.skipped === null)).find(Boolean) ??
      headlineMeasures.map((m) => amounts.find((a) => a.amount.measure === m && a.skipped === "included_in")).find(Boolean) ??
      null;
    const headlineQuantity = quantities.find((q) => q.quantity.measure !== "shares" && q.quantity.measure !== "investors") ?? null;
    const headline = headlineAmount
      ? {
          short: headlineAmount.short,
          words: headlineAmount.words,
          kind: "amount" as const,
          countedElsewhere: headlineAmount.skipped === "included_in",
          amount: headlineAmount,
          source: headlineAmount.source,
        }
      : headlineQuantity
        ? { short: headlineQuantity.short, words: headlineQuantity.short, kind: "quantity" as const, countedElsewhere: false, amount: null, source: headlineQuantity.source }
        : null;

    const side = (roles: Role[]) => participants.filter((p) => p.roles.some((r) => roles.includes(r)));
    const buyers = side(["buyer", "lessee", "acquirer"]);
    const providers = side(["provider", "lessor", "target", "seller"]);
    const partiesText =
      eventRoute(event) === "deals" && buyers.length > 0 && providers.length > 0
        ? `${buyers.map((p) => p.org.displayName).join(", ")} ↔ ${providers.map((p) => p.org.displayName).join(", ")}`
        : (issuerEntry?.org.displayName ?? participants[0].org.displayName);

    const view: EventView = {
      ...refOf(event),
      event,
      route: eventRoute(event),
      ogImage: eventRoute(event) === "rounds" ? roundOgImagePath(event.slug) : dealOgImagePath(event.slug),
      tag: feedTag(event.kind === "deal" && isCompute ? event.types.filter((t) => COMPUTE_TYPES.has(t)) : event.types),
      typeLabels: event.types.map((t) => EVENT_TYPE_LABELS[t]),
      statusLabel: EVENT_STATUS_LABELS[event.status],
      issuer: issuerEntry?.org ?? null,
      participants,
      amounts,
      quantities,
      facts: event.facts.map((f) => ({ ...f, source: sourceOf(f.sourceId) })),
      sources: eventSources,
      primary,
      indexable: primary,
      headline,
      valuation: amounts.find((a) => a.amount.measure === "valuation_post") ?? null,
      partiesText,
      relations: event.relations.map((r) => ({ type: r.type, note: r.note ?? null, event: refOf(events.get(r.event)!) })),
      isCompute,
    };
    eventViewCache.set(event.id, view);
    return view;
  }

  const allEvents = () => [...events.values()].map(eventView).sort(byDateDesc);
  const eventsOf = (orgId: string) => allEvents().filter((e) => e.event.participants.some((p) => p.org === orgId));

  /** Closed primary equity rounds where the org raised money, per currency. */
  function equityRaised(orgId: string): { totals: LedgerTotal[]; rounds: number } {
    const amounts = [...events.values()]
      .filter((e) => e.kind === "financing" && e.types.includes("equity_round"))
      .flatMap((e) => e.amounts)
      .filter((a) => a.measure === "equity_raised" && a.to === orgId && a.status === "closed");
    const totals = totalsByCurrency(amounts, "equity_raised").filter((t) => t.value !== null);
    return { totals, rounds: totals.reduce((n, t) => n + t.counted.length, 0) };
  }

  const capexViews = new Map<string, CapexView>(content.capex.series.map((s) => [s.org, deriveCapex(s)] as const));

  function headerAmount(orgId: string, measure: Measure, role: "to" | "from" | "issuer" = "to"): AmountView | null {
    const candidates = [...events.values()]
      .filter((e) => e.participants.some((p) => p.org === orgId))
      .flatMap((e) => e.amounts.map((a) => ({ a, e })))
      .filter(({ a, e }) => {
        if (a.measure !== measure || !HEADER_STATUSES.has(a.status) || a.value === null) return false;
        if (role === "issuer") return e.participants.some((p) => p.org === orgId && p.roles.includes("issuer"));
        return role === "to" ? a.to === orgId : a.from === orgId;
      })
      .sort((x, y) => y.a.asOf.localeCompare(x.a.asOf) || y.e.date.localeCompare(x.e.date));
    const top = candidates[0];
    return top ? amountView(top.a, top.e) : null;
  }

  function statementsOf(orgId: string): Array<Statement & { source: Source; short: string; statusLabel: string }> {
    return statements
      .filter((s) => s.org === orgId)
      .sort((a, b) => b.asOf.localeCompare(a.asOf) || a.id.localeCompare(b.id))
      .map((s) => ({
        ...s,
        source: sourceOf(s.sourceId),
        short: formatAmountShort(s),
        statusLabel: s.status === "reported" ? `Reported by ${sourceOf(s.sourceId).publisher}` : AMOUNT_STATUS_LABELS[s.status],
      }));
  }

  function company(slug: string) {
    const org = [...orgs.values()].find((o) => o.slug === slug && o.kinds.includes("company"));
    if (!org) return null;
    const view = orgView(org.id);
    const timeline = eventsOf(org.id);
    const rounds = timeline.filter((e) => isCompanyFinancing(e.event, org.id));
    const compute = timeline.filter((e) => e.isCompute);
    const otherDeals = timeline.filter((e) => e.route === "deals" && !e.isCompute);
    const asInvestor = timeline.filter(
      (e) => isFinancing(e.event) && e.event.participants.some((p) => p.org === org.id && p.roles.some((r) => INVESTOR_ROLES.has(r)))
    );
    const investors = new Map<string, { org: OrgView; roles: Set<Role>; events: EventRef[] }>();
    for (const round of rounds) {
      for (const p of round.participants) {
        if (p.org.id === org.id || !p.roles.some((r) => INVESTOR_ROLES.has(r))) continue;
        const entry = investors.get(p.org.id) ?? { org: p.org, roles: new Set<Role>(), events: [] };
        p.roles.forEach((r) => entry.roles.add(r));
        entry.events.push(round);
        investors.set(p.org.id, entry);
      }
    }
    // Strategic investors also appear through deals where this company receives the money (Amazon's
    // April 2026 investment in Anthropic). Investors in someone else's round are not this company's investors.
    for (const deal of timeline) {
      if (rounds.includes(deal)) continue;
      const receives = deal.event.participants.some((p) => p.org === org.id && p.roles.includes("issuer"));
      if (!receives) continue;
      for (const p of deal.participants) {
        if (p.org.id === org.id || !p.roles.includes("strategic_investor")) continue;
        const entry = investors.get(p.org.id) ?? { org: p.org, roles: new Set<Role>(), events: [] };
        entry.roles.add("strategic_investor");
        if (!entry.events.includes(deal)) entry.events.push(deal);
        investors.set(p.org.id, entry);
      }
    }
    const metricStatements = statementsOf(org.id);
    const capex = capexViews.get(org.id) ?? null;
    // Parties to deals (compute, leases, acquisitions). Round investors are listed in their own section.
    const counterparties = new Map<string, OrgView>();
    for (const e of timeline.filter((x) => x.route === "deals")) {
      for (const p of e.participants) if (p.org.id !== org.id) counterparties.set(p.org.id, p.org);
    }
    const related = [...orgs.values()]
      .filter((o) => o.id !== org.id && o.kinds.includes("company") && view.primaryCategory && o.categories.includes(view.primaryCategory))
      .map((o) => orgView(o.id));
    const citedSourceIds = new Set<string>([
      ...timeline.flatMap((e) => e.event.sourceIds),
      ...metricStatements.map((s) => s.sourceId),
      ...(org.parent ? [org.parent.sourceId] : []),
      ...(org.founded ? [org.founded.sourceId] : []),
    ]);
    const factCount = timeline.length + metricStatements.length + (capex ? capex.quarters.length + capex.years.length : 0);
    const reviewed = [org.reviewedAt, ...timeline.map((e) => e.event.reviewedAt), ...timeline.flatMap((e) => e.event.corrections.map((c) => c.date))]
      .sort()
      .at(-1)!;
    const latestRound = rounds.find((r) => r.event.types.includes("equity_round")) ?? null;
    const subsidiaries = [...orgs.values()].filter((o) => o.parent?.org === org.id).map((o) => orgView(o.id));

    return {
      org: view,
      template: view.primaryCategory ? TEMPLATE_BY_CATEGORY[view.primaryCategory] : ("lab" as ProfileTemplate),
      parent: org.parent ? { org: orgView(org.parent.org), since: org.parent.since, source: sourceOf(org.parent.sourceId), quote: org.parent.quote } : null,
      subsidiaries,
      founded: org.founded ? { date: org.founded.date, source: sourceOf(org.founded.sourceId) } : null,
      timeline,
      rounds,
      latestRound,
      latestValuation: headerAmount(org.id, "valuation_post", "issuer"),
      equityRaised: equityRaised(org.id),
      investors: [...investors.values()].map((i) => ({ ...i, roles: [...i.roles] })),
      asInvestor,
      compute,
      otherDeals,
      statements: metricStatements,
      latestRunRate: metricStatements.find((s) => s.measure === "revenue_run_rate" && HEADER_STATUSES.has(s.status)) ?? null,
      capex,
      counterparties: [...counterparties.values()],
      related,
      sources: [...citedSourceIds].map(sourceOf).sort((a, b) => (b.publishedAt ?? "").localeCompare(a.publishedAt ?? "")),
      factCount,
      indexable: factCount > 0,
      reviewedAt: reviewed,
    };
  }

  function investor(slug: string) {
    const org = [...orgs.values()].find((o) => o.slug === slug && o.kinds.includes("investor"));
    if (!org) return null;
    const view = orgView(org.id);
    const participations = (investorParticipations.get(org.id) ?? [])
      .map(({ event, roles }) => ({ event: eventView(event), roles, roleLabels: roles.map((r) => ROLE_LABELS[r]) }))
      .sort((a, b) => byDateDesc(a.event, b.event));
    const portfolio = new Map<string, OrgView>();
    for (const p of participations) {
      const issuer = p.event.issuer;
      if (issuer && issuer.id !== org.id) portfolio.set(issuer.id, issuer);
    }
    const ownAmounts = participations.flatMap((p) =>
      p.event.amounts.filter((a) => a.amount.from === org.id && a.amount.measure !== "cumulative_investment")
    );
    const otherEvents = eventsOf(org.id).filter((e) => !participations.some((p) => p.event.id === e.id));
    const factCount = participations.length;
    const dates = participations.map((p) => p.event.date).sort();
    return {
      org: view,
      participations,
      leadCount: participations.filter((p) => p.roles.includes("lead_investor") || p.roles.includes("co_lead")).length,
      firstDate: dates[0] ?? null,
      latestDate: dates.at(-1) ?? null,
      portfolio: [...portfolio.values()],
      ownAmounts,
      otherEvents,
      companyHref: view.companyHref,
      sources: [...new Set(participations.flatMap((p) => p.event.event.sourceIds))].map(sourceOf),
      factCount,
      indexable: factCount > 0,
      reviewedAt: [org.reviewedAt, ...participations.map((p) => p.event.event.reviewedAt)].sort().at(-1)!,
    };
  }

  /* ----------------------------------------------------------- overview */

  function equityYtd(year: string) {
    const amounts = [...events.values()]
      .filter((e) => e.kind === "financing" && e.types.includes("equity_round") && e.date.startsWith(year))
      .filter((e) => e.participants.some((p) => p.roles.includes("issuer") && orgs.get(p.org)?.categories.includes("ai-lab")))
      .flatMap((e) => e.amounts)
      .filter((a) => a.measure === "equity_raised" && a.status === "closed");
    return totalsByCurrency(amounts, "equity_raised").filter((t) => t.value !== null);
  }

  function computeYear(year: string) {
    const deals = allEvents().filter((e) => e.isCompute && e.date.startsWith(year));
    const values = deals.flatMap((d) =>
      d.amounts.filter(
        (a) =>
          (a.amount.measure === "contract_value" || a.amount.measure === "compute_commitment") &&
          a.skipped === null &&
          a.amount.value !== null &&
          HEADER_STATUSES.has(a.amount.status)
      )
    );
    const largest = [...values].sort((a, b) => (b.amount.value ?? 0) - (a.amount.value ?? 0))[0] ?? null;
    const measures = new Set(values.map((v) => v.amount.measure));
    const summed =
      measures.size === 1 && values.length > 0
        ? tryTotal(values.map((v) => v.amount), { measure: values[0].amount.measure, currency: values[0].amount.currency })
        : { ok: false as const, reason: measures.size > 1 ? "contract values and compute commitments are different measures" : "no stated values" };
    const gw = deals.flatMap((d) => d.quantities.filter((q) => q.quantity.measure === "capacity_gw"));
    return { deals, largest, summed, withValue: values.length, gwDeals: gw.length };
  }

  /** Latest calendar quarter with cash capex from at least `min` companies, summed only within aligned USD quarters. */
  function capexCalendarQuarter(min = 3) {
    const byKey = new Map<string, Array<{ org: OrgView; quarter: CapexQuarter }>>();
    for (const [orgId, view] of capexViews) {
      for (const q of view.quarters) {
        if (!q.calendarQuarter) continue;
        const list = byKey.get(q.calendarQuarter) ?? [];
        list.push({ org: orgView(orgId), quarter: q });
        byKey.set(q.calendarQuarter, list);
      }
    }
    const key = [...byKey.keys()].sort().reverse().find((k) => byKey.get(k)!.filter((e) => e.quarter.currency === "USD").length >= min);
    if (!key) return null;
    const entries = byKey.get(key)!.filter((e) => e.quarter.currency === "USD");
    const amounts: Amount[] = entries.map((e) => ({
      id: `${e.org.id}_${key}`.replace(/[^a-z0-9_]/g, "_"),
      measure: "capex_actual",
      value: e.quarter.value,
      currency: "USD",
      basis: "exact",
      status: "filed",
      asOf: e.quarter.end,
      sourceId: "src_capex",
      quote: "XBRL fact",
    }));
    const sum = total(amounts, { measure: "capex_actual", currency: "USD" });
    return {
      key,
      label: quarterLabel(key),
      entries: entries.sort((a, b) => b.quarter.value - a.quarter.value),
      total: sum,
      derivedCount: entries.filter((e) => e.quarter.derived).length,
    };
  }

  function largestValuation() {
    const candidates = [...events.values()]
      .flatMap((e) => e.amounts.map((a) => ({ a, e })))
      .filter(({ a }) => a.measure === "valuation_post" && a.value !== null && HEADER_STATUSES.has(a.status))
      .filter(({ e }) => hasPrimarySource(e.sourceIds, sources))
      .sort((x, y) => (y.a.value ?? 0) - (x.a.value ?? 0) || y.e.date.localeCompare(x.e.date));
    const top = candidates[0];
    return top ? { amount: amountView(top.a, top.e), event: eventView(top.e) } : null;
  }

  function companies() {
    return [...orgs.values()].filter((o) => o.kinds.includes("company")).map((o) => orgView(o.id));
  }

  function investorsList() {
    return [...orgs.values()]
      .filter((o) => o.kinds.includes("investor"))
      .map((o) => {
        const parts = (investorParticipations.get(o.id) ?? []).sort((a, b) => b.event.date.localeCompare(a.event.date));
        return {
          org: orgView(o.id),
          rounds: parts.length,
          leads: parts.filter((p) => p.roles.includes("lead_investor") || p.roles.includes("co_lead")).length,
          latest: parts[0] ? eventView(parts[0].event) : null,
          portfolio: [...new Set(parts.map((p) => eventView(p.event).issuer?.displayName).filter(Boolean))] as string[],
          indexable: parts.length > 0,
        };
      });
  }

  function sector(slug: string) {
    const hub = SECTORS.find((s) => s.slug === slug);
    if (!hub) return null;
    const members = [...orgs.values()].filter((o) => o.kinds.includes("company") && o.categories.some((c) => hub.categories.includes(c)));
    const ids = new Set(members.map((o) => o.id));
    const hubEvents = allEvents().filter((e) => e.event.participants.some((p) => ids.has(p.org)));
    return {
      ...hub,
      href: sectorPath(hub.slug),
      plural: CATEGORY_PLURALS[hub.categories[0]],
      companies: members.map((o) => orgView(o.id)),
      events: hubEvents,
      capex: members.map((o) => capexViews.get(o.id)).filter((v): v is CapexView => Boolean(v)),
    };
  }

  /** Previous equity round of the same issuer that states a post-money valuation. */
  function previousRound(e: EventView): EventView | null {
    if (!e.issuer || !e.event.types.includes("equity_round")) return null;
    return (
      allEvents()
        .filter(
          (r) =>
            r.id !== e.id &&
            r.route === "rounds" &&
            r.issuer?.id === e.issuer?.id &&
            r.event.types.includes("equity_round") &&
            r.date < e.date &&
            r.valuation !== null &&
            r.valuation.amount.status !== "reported"
        )
        .sort((a, b) => b.date.localeCompare(a.date))[0] ?? null
    );
  }

  /**
   * Step-up in post-money valuation vs the previous round, computed by Tokscale. Only between
   * two sourced, non-press valuations in the same currency. Bounds/approximations survive;
   * incompatible bounds return null rather than pretending their thresholds are exact values.
   */
  function stepUp(e: EventView): (ValuationRatio & { previous: EventView; formula: string }) | null {
    const previous = previousRound(e);
    const now = e.valuation?.amount;
    const before = previous?.valuation?.amount;
    if (!previous || !now || !before || now.status === "reported") return null;
    const result = valuationRatio(now, before);
    return result ? { ...result, previous, formula: `${formatAmountShort(now)} ÷ ${formatAmountShort(before)}` } : null;
  }

  /** One row per company for the directory. Pure data; the page joins Tokscale usage. */
  function directoryRows() {
    return companies().map((org) => {
      const timeline = eventsOf(org.id);
      const financing = timeline.filter((e) => isCompanyFinancing(e.event, org.id));
      const lastRound = financing.find((e) => e.event.types.includes("equity_round")) ?? null;
      const lastFinancing = financing[0] ?? null;
      const valuation = headerAmount(org.id, "valuation_post", "issuer");
      const equity = equityRaised(org.id);
      const compute = timeline.filter((e) => e.isCompute);
      const asBuyer = compute.filter((e) => e.event.participants.some((p) => p.org === org.id && p.roles.some((r) => r === "buyer" || r === "lessee")));
      const asProvider = compute.filter((e) => e.event.participants.some((p) => p.org === org.id && p.roles.some((r) => r === "provider" || r === "lessor")));
      const contractValues = compute
        .flatMap((e) => e.amounts)
        .filter((a) => (a.amount.measure === "contract_value" || a.amount.measure === "compute_commitment") && a.skipped === null && a.amount.value !== null && HEADER_STATUSES.has(a.amount.status));
      const largestContract = [...contractValues].sort((a, b) => (b.amount.value ?? 0) - (a.amount.value ?? 0))[0] ?? null;
      const mw = compute.flatMap((e) => e.quantities).filter((q) => q.quantity.measure === "it_load_mw");
      const gw = compute.flatMap((e) => e.quantities).filter((q) => q.quantity.measure === "capacity_gw");
      const capex = capexViews.get(org.id) ?? null;
      const lastFour = capex ? capex.quarters.slice(-4) : [];
      const ttmContiguous =
        lastFour.length === 4 &&
        lastFour.every((q, i) => i === 0 || Date.parse(`${q.start}T00:00:00Z`) - Date.parse(`${lastFour[i - 1].end}T00:00:00Z`) <= 86_400_000 * 2);
      const formD = financing
        .flatMap((e) => e.amounts)
        .find((a) => a.amount.measure === "form_d_amount_sold" && a.amount.value !== null) ?? null;
      return {
        org,
        country: org.org.hq?.country ?? null,
        categories: org.org.categories,
        lastFinancing,
        lastRound,
        roundsCount: financing.filter((e) => e.event.types.includes("equity_round")).length,
        valuation,
        equity,
        dealsCount: compute.length,
        dealsAsBuyer: asBuyer.length,
        dealsAsProvider: asProvider.length,
        largestContract,
        capacityMw: tryQuantityTotal(mw.map((q) => q.quantity), "it_load_mw"),
        capacityGw: tryQuantityTotal(gw.map((q) => q.quantity), "capacity_gw"),
        lastActivity: timeline[0]?.date ?? null,
        capexLatest: capex?.quarters.at(-1) ?? null,
        capexTtm: ttmContiguous ? { value: lastFour.reduce((s, q) => s + q.value, 0), currency: lastFour[0].currency, start: lastFour[0].start, end: lastFour[3].end, derived: lastFour.some((q) => q.derived) } : null,
        formD,
        leadsOfLastRound: lastRound
          ? lastRound.participants.filter((p) => p.roles.includes("lead_investor") || p.roles.includes("co_lead")).map((p) => p.org)
          : [],
      };
    });
  }

  const reviewedAt = () =>
    [
      ...[...events.values()].map((e) => e.reviewedAt),
      ...[...events.values()].flatMap((e) => e.corrections.map((c) => c.date)),
      ...[...orgs.values()].map((o) => o.reviewedAt),
    ]
      .sort()
      .at(-1)!;

  return {
    content,
    orgView,
    eventView,
    getEvents: allEvents,
    getEvent: (slug: string) => {
      const event = [...events.values()].find((e) => e.slug === slug);
      return event ? eventView(event) : null;
    },
    // Route selectors are for static params. Category indexes can include events on either route.
    getRounds: () => allEvents().filter((e) => e.route === "rounds"),
    getFinancings: () => allEvents().filter((e) => isFinancing(e.event)),
    getDeals: () => allEvents().filter((e) => e.route === "deals"),
    getComputeDeals: () => allEvents().filter((e) => e.isCompute),
    getCompanies: companies,
    getCompany: company,
    getDirectoryRows: directoryRows,
    previousRound,
    stepUp,
    getInvestors: investorsList,
    getInvestor: investor,
    getSector: sector,
    getSectors: () => SECTORS.map((s) => sector(s.slug)!),
    getCapex: () =>
      [...capexViews.entries()]
        .map(([orgId, view]) => ({ org: orgView(orgId), view }))
        .sort((a, b) => (b.view.quarters.at(-1)?.value ?? 0) - (a.view.quarters.at(-1)?.value ?? 0)),
    getCapexFor: (orgId: string) => capexViews.get(orgId) ?? null,
    capexRetrievedAt: content.capex.retrievedAt,
    capexDefinition: content.capex.definition,
    equityRaised,
    equityYtd,
    computeYear,
    capexCalendarQuarter,
    largestValuation,
    statementsOf,
    reviewedAt,
    sourceOf,
    formatMoney,
  };
}

export type LedgerModel = ReturnType<typeof createLedgerModel>;
export type CompanyProfile = NonNullable<ReturnType<LedgerModel["getCompany"]>>;
export type InvestorProfile = NonNullable<ReturnType<LedgerModel["getInvestor"]>>;
export type InvestorRow = ReturnType<LedgerModel["getInvestors"]>[number];
export type SectorView = NonNullable<ReturnType<LedgerModel["getSector"]>>;
export type DirectoryRow = ReturnType<LedgerModel["getDirectoryRows"]>[number];

export const ledger: LedgerModel = createLedgerModel(ledgerContent);

export type { InvestorType };
