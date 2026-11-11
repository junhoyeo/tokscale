/**
 * Data for the Capital section on the home page: the four money types (the same
 * functions the /capital overview uses), the AI labs behind the models Tokscale
 * users run, and the latest events.
 *
 * Everything is plain, preformatted data so the client component only renders it.
 * Money types are never added together; lab usage is labelled as Tokscale usage,
 * not market share.
 */
import { computePath, roundsPath, spendingPath } from "../urls";
import { formatMonthYear, formatPartialDate, formatTotal, sourceShortLabel } from "./format";
import { ledger as defaultLedger, type LedgerModel } from "./model";
import { loadUsageSnapshot, usageForLabs, type UsageSnapshot } from "./usage";

export interface HomeCapitalStat {
  label: string;
  value: string;
  detail: string;
  href: string;
  linkLabel: string;
}

export interface HomeCapitalLab {
  slug: string;
  name: string;
  href: string | null;
  logo: string | null;
  avatar: string | null;
  monogram: string;
  /** Share of identified-model tokens among Tokscale users in the window (0..1). */
  share: number;
  topModel: { key: string; name: string; logoUrl: string | null; href: string } | null;
  /** "Series H", with its post-money valuation when stated. */
  round: { label: string; date: string; href: string; valuation: string | null } | null;
  /** Shown when there is no round, e.g. "Public · NASDAQ: GOOGL". */
  status: string;
  /** Largest stated compute contract value, with the other party. */
  compute: { value: string; with: string; href: string | null } | null;
}

export interface HomeCapitalEvent {
  id: string;
  date: string;
  tag: string;
  title: string;
  parties: string;
  value: string | null;
  valuation: string | null;
  href: string;
  source: { label: string; url: string } | null;
}

export interface HomeCapital {
  stats: HomeCapitalStat[];
  labs: HomeCapitalLab[];
  /** Window of the lab usage, e.g. "Sep 20 – Oct 19, 2026"; null without usage data. */
  labsWindow: { start: string; end: string; accounts: number } | null;
  events: HomeCapitalEvent[];
  reviewedAt: string;
}

const LAB_LIMIT = 5;
const EVENT_LIMIT = 4;

function stats(ledger: LedgerModel, year: string): HomeCapitalStat[] {
  const equity = ledger.equityYtd(year);
  const usd = equity.find((t) => t.currency === "USD") ?? null;
  const other = equity.filter((t) => t.currency !== "USD");
  const compute = ledger.computeYear(year);
  const capex = ledger.capexCalendarQuarter();
  const valuation = ledger.largestValuation();

  const equityDetail = usd
    ? `${usd.counted.length} closed AI-lab ${usd.counted.length === 1 ? "round" : "rounds"}${other
        .map((t) => ` · plus ${formatTotal(t)} in ${t.currency}, not converted`)
        .join("")}`
    : "No closed AI-lab rounds tracked this year";

  const computeStat: HomeCapitalStat =
    compute.summed.ok && compute.summed.total.value !== null
      ? {
          label: `Compute deals · ${year}`,
          value: formatTotal(compute.summed.total),
          detail: `${compute.summed.total.counted.length} stated contract values, multi-year totals`,
          href: computePath(),
          linkLabel: "Compute",
        }
      : {
          label: `Compute deals · ${year}`,
          value: `${compute.deals.length} ${compute.deals.length === 1 ? "deal" : "deals"}`,
          detail: compute.largest
            ? `Largest stated value ${compute.largest.short}${compute.largest.event ? `, ${ledger.eventView(compute.largest.event).partiesText}` : ""}`
            : "No stated contract values this year",
          href: computePath(),
          linkLabel: "Compute",
        };

  return [
    {
      label: `Equity raised · ${year} so far`,
      value: usd ? formatTotal(usd) : "—",
      detail: equityDetail,
      href: roundsPath(),
      linkLabel: "Rounds",
    },
    computeStat,
    {
      label: `Cash capex · ${capex ? capex.label : "latest quarter"}`,
      value: capex ? formatTotal(capex.total) : "—",
      detail: capex
        ? `${capex.entries.length} companies, cash paid for property and equipment`
        : "Not enough companies have reported the same quarter yet",
      href: spendingPath(),
      linkLabel: "Spending",
    },
    {
      label: "Largest valuation",
      value: valuation ? valuation.amount.short : "—",
      detail: valuation
        ? `${valuation.event.issuer?.name ?? valuation.event.partiesText}, post-money, ${formatPartialDate(valuation.event.date)}`
        : "No valuation tracked",
      href: valuation ? valuation.event.href : roundsPath(),
      linkLabel: valuation ? valuation.event.title : "Rounds",
    },
  ];
}

function labs(ledger: LedgerModel, snapshot: UsageSnapshot | null): HomeCapitalLab[] {
  if (!snapshot) return [];
  const rows = ledger.getDirectoryRows().filter((row) => row.org.org.labSlugs.length > 0);
  const withUsage = rows
    .map((row) => ({ row, usage: usageForLabs(snapshot, row.org.org.labSlugs) }))
    .filter((x) => x.usage !== null && x.usage.tokens > 0 && x.usage.share !== null)
    .sort((a, b) => b.usage!.tokens - a.usage!.tokens || a.row.org.name.localeCompare(b.row.org.name))
    .slice(0, LAB_LIMIT);

  return withUsage.map(({ row, usage }) => {
    const top = usage!.models[0] ?? null;
    const round = row.lastRound;
    const contract = row.largestContract;
    let compute: HomeCapitalLab["compute"] = null;
    if (contract) {
      const ev = contract.event ? ledger.eventView(contract.event) : null;
      const others = ev ? ev.participants.filter((p) => p.org.id !== row.org.id).map((p) => p.org.displayName) : [];
      compute = { value: contract.short, with: others.join(", "), href: ev ? ev.href : null };
    }
    return {
      slug: row.org.slug,
      name: row.org.displayName,
      href: row.org.companyHref ?? row.org.href,
      logo: row.org.logo,
      avatar: row.org.avatar,
      monogram: row.org.monogram,
      share: usage!.share ?? 0,
      topModel: top ? { key: top.key, name: top.name, logoUrl: top.logoUrl, href: `/models/${encodeURIComponent(top.key)}` } : null,
      round: round
        ? {
            label: round.event.roundLabel ?? round.typeLabels[0] ?? "Equity round",
            date: formatMonthYear(round.date),
            href: round.href,
            valuation: round.valuation ? round.valuation.short : row.valuation && row.valuation.event?.id === round.event.id ? row.valuation.short : null,
          }
        : null,
      status: row.org.statusText,
      compute,
    };
  });
}

function events(ledger: LedgerModel): HomeCapitalEvent[] {
  return ledger
    .getEvents()
    .slice(0, EVENT_LIMIT)
    .map((e) => {
      const source = e.headline?.source ?? e.sources[0];
      return {
        id: e.id,
        date: formatPartialDate(e.date),
        tag: e.tag,
        title: e.title,
        parties: e.partiesText,
        value: e.headline ? e.headline.short : null,
        valuation: e.valuation ? e.valuation.short : null,
        href: e.href,
        source: source ? { label: sourceShortLabel(source), url: source.url } : null,
      };
    });
}

/** Pure builder (tests pass their own ledger and usage snapshot). */
export function buildHomeCapital(input: {
  ledger?: LedgerModel;
  snapshot: UsageSnapshot | null;
}): HomeCapital {
  const ledger = input.ledger ?? defaultLedger;
  const reviewedAt = ledger.reviewedAt();
  const year = reviewedAt.slice(0, 4);
  const snapshot = input.snapshot;
  return {
    stats: stats(ledger, year),
    labs: labs(ledger, snapshot),
    labsWindow: snapshot ? { start: snapshot.coverage.start, end: snapshot.coverage.end, accounts: snapshot.coverage.accounts } : null,
    events: events(ledger),
    reviewedAt,
  };
}

/** Home page data. Tokscale usage is optional: without it the lab table is omitted. */
export async function getHomeCapital(): Promise<HomeCapital> {
  const snapshot = await loadUsageSnapshot();
  return buildHomeCapital({ snapshot });
}

