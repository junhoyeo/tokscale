import type { CapexFact, CapexSeries } from "./schema";

/**
 * Capex views derived from SEC XBRL facts ("cash paid for property and equipment").
 *
 * Pure functions: no I/O. Input is one series from metrics/capex.json (see capexSeriesSchema).
 *
 * 10-Q cash flows are year to date, so most filed facts span 6 or 9 months. A quarter is either a
 * filed ~3-month fact (direct) or the difference of two year-to-date facts of the same fiscal year
 * (derived): Q2 = 6M - Q1, Q3 = 9M - 6M, Q4 = 12M - 9M. Nothing is derived across a gap, a missing
 * figure is absent (never zero), and a derived value below zero is dropped with a warning.
 */

export interface CapexSourceRef {
  accession: string;
  form: string;
  filed: string;
  url: string;
}

export interface CapexQuarter {
  org: string;
  fiscalYear: number;
  fiscalQuarter: 1 | 2 | 3 | 4;
  start: string;
  end: string;
  value: number;
  currency: string;
  /** True when computed from year-to-date facts. */
  derived: boolean;
  /** Plain words, for example "Six months to Jun 30, 2026 ($X) minus three months to Mar 31, 2026 ($Y)". */
  formula: string | null;
  /** Every filing used. */
  sources: CapexSourceRef[];
  /** "2026-q2" only when the span is a calendar quarter; otherwise null. */
  calendarQuarter: string | null;
}

export interface CapexYear {
  org: string;
  fiscalYear: number;
  start: string;
  end: string;
  value: number;
  currency: string;
  source: CapexSourceRef;
}

export interface CapexView {
  org: string;
  currency: string;
  fiscalYearEnd: string;
  lineItem: string | null;
  notes: string[];
  /** Oldest first. */
  quarters: CapexQuarter[];
  /** Oldest first, from 12-month annual facts. */
  years: CapexYear[];
  /** Data problems found while deriving (for example direct quarters that do not add up). */
  warnings: string[];
}

/** One filed fact placed in its fiscal year (see capexPeriods). */
export interface CapexPeriod {
  org: string;
  fiscalYear: number;
  /** 3 for a quarter; 6, 9 or 12 for a fiscal year-to-date span. */
  months: 3 | 6 | 9 | 12;
  start: string;
  end: string;
  value: number;
  currency: string;
  /** "Six months to Jun 30, 2026", "Three months to Mar 31, 2026". */
  label: string;
  source: CapexSourceRef;
}

/* ------------------------------------------------------------- dates */

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"] as const;
const DAY_MS = 86_400_000;
/** Tolerance for 52/53-week fiscal years, whose year ends drift around the month-day in the SEC record. */
const FISCAL_YEAR_END_TOLERANCE_DAYS = 7;

interface Ymd {
  y: number;
  m: number; // 1..12
  d: number;
}

function parseIso(iso: string): Ymd {
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(iso);
  if (!match) throw new Error(`expected YYYY-MM-DD, got ${iso}`);
  return { y: Number(match[1]), m: Number(match[2]), d: Number(match[3]) };
}

/** Days since 1970-01-01 (UTC). */
function dayNumber(iso: string): number {
  const { y, m, d } = parseIso(iso);
  return Math.round(Date.UTC(y, m - 1, d) / DAY_MS);
}

function isoOfDay(day: number): string {
  return new Date(day * DAY_MS).toISOString().slice(0, 10);
}

function daysInMonth(y: number, m: number): number {
  return new Date(Date.UTC(y, m, 0)).getUTCDate();
}

/** "Jun 30, 2026" */
function longDate(iso: string): string {
  const { y, m, d } = parseIso(iso);
  return `${MONTHS[m - 1]} ${d}, ${y}`;
}

/* ------------------------------------------------------------ helpers */

/** Months covered by a span: 3 for 80-100 days, 6, 9 or 12 for year-to-date lengths; null otherwise. */
export function spanMonths(start: string, end: string): 3 | 6 | 9 | 12 | null {
  const days = dayNumber(end) - dayNumber(start);
  if (days >= 80 && days <= 100) return 3;
  if (days >= 170 && days <= 200) return 6;
  if (days >= 260 && days <= 290) return 9;
  if (days >= 350 && days <= 380) return 12;
  return null;
}

/**
 * True when `date` is the day after a fiscal year end ("MM-DD"), within 7 days for 52/53-week years.
 * Amazon-style trailing-twelve-month rows start mid-year and fail this test.
 */
export function isFiscalYearStart(date: string, fiscalYearEnd: string): boolean {
  const match = /^(\d{2})-(\d{2})$/.exec(fiscalYearEnd);
  if (!match) throw new Error(`expected MM-DD, got ${fiscalYearEnd}`);
  const month = Number(match[1]);
  const day = Number(match[2]);
  const previous = dayNumber(date) - 1;
  const year = parseIso(isoOfDay(previous)).y;
  for (const y of [year - 1, year, year + 1]) {
    const fyEnd = Math.round(Date.UTC(y, month - 1, Math.min(day, daysInMonth(y, month))) / DAY_MS);
    if (Math.abs(previous - fyEnd) <= FISCAL_YEAR_END_TOLERANCE_DAYS) return true;
  }
  return false;
}

/** "FY26 Q3" */
export function fiscalLabel(q: { fiscalYear: number; fiscalQuarter: number }): string {
  return `FY${String(q.fiscalYear % 100).padStart(2, "0")} Q${q.fiscalQuarter}`;
}

/**
 * Calendar months a span covers: "Jan–Mar 2026", "Dec 2025–Feb 2026" (en dash).
 * A start in the last 7 days of a month counts from the next month, and an end in the first 7 days
 * of a month counts to the previous month, so 52/53-week quarters read naturally
 * (Jan 26–Apr 26, 2026 is "Feb–Apr 2026").
 */
export function calendarSpanLabel(start: string, end: string): string {
  const s = parseIso(start);
  const e = parseIso(end);
  let sy = s.y;
  let sm = s.m;
  let ey = e.y;
  let em = e.m;
  if (s.d > daysInMonth(s.y, s.m) - 7) {
    sm += 1;
    if (sm > 12) {
      sm = 1;
      sy += 1;
    }
  }
  if (e.d <= 7) {
    em -= 1;
    if (em < 1) {
      em = 12;
      ey -= 1;
    }
  }
  if (sy * 12 + sm > ey * 12 + em) {
    // Too short to snap (a span of a few days): use the raw months.
    sy = s.y;
    sm = s.m;
    ey = e.y;
    em = e.m;
  }
  if (sy === ey && sm === em) return `${MONTHS[sm - 1]} ${sy}`;
  if (sy === ey) return `${MONTHS[sm - 1]}–${MONTHS[em - 1]} ${sy}`;
  return `${MONTHS[sm - 1]} ${sy}–${MONTHS[em - 1]} ${ey}`;
}

/**
 * "2026-q2" when the span is a calendar quarter: it starts within the first 7 days of Jan, Apr, Jul
 * or Oct and ends within the last 7 days of Mar, Jun, Sep or Dec of the same quarter. Otherwise null.
 */
export function calendarQuarterOf(start: string, end: string): string | null {
  const s = parseIso(start);
  const e = parseIso(end);
  if ((s.m - 1) % 3 !== 0 || s.d > 7) return null;
  if (e.y !== s.y || e.m !== s.m + 2) return null;
  if (e.d <= daysInMonth(e.y, e.m) - 7) return null;
  return `${s.y}-q${(s.m - 1) / 3 + 1}`;
}

/** https://www.sec.gov/Archives/edgar/data/<cik without leading zeros>/<accession without dashes>/<accession>-index.htm */
export function secFilingIndexUrl(cik: string, accession: string): string {
  const plainCik = cik.replace(/^0+(?=\d)/, "");
  return `https://www.sec.gov/Archives/edgar/data/${plainCik}/${accession.replace(/-/g, "")}/${accession}-index.htm`;
}

/** "$28,476,000,000" for USD; "TWD 956,006,500,000" for other currencies. Exact, no rounding. */
export function formatCapexAmount(value: number, currency: string): string {
  const sign = value < 0 ? "-" : "";
  const [whole, fraction] = String(Math.abs(value)).split(".");
  const grouped = whole.replace(/\B(?=(\d{3})+(?!\d))/g, ",") + (fraction ? `.${fraction}` : "");
  return currency === "USD" ? `${sign}$${grouped}` : `${sign}${currency} ${grouped}`;
}

const SPAN_WORDS: Record<3 | 6 | 9 | 12, string> = { 3: "three months", 6: "six months", 9: "nine months", 12: "twelve months" };

/** "six months to Jun 30, 2026" */
function spanWords(months: 3 | 6 | 9 | 12, end: string): string {
  return `${SPAN_WORDS[months]} to ${longDate(end)}`;
}

/** "Six months to Jun 30, 2026" */
export function capexSpanLabel(months: 3 | 6 | 9 | 12, end: string): string {
  const words = spanWords(months, end);
  return `${words[0].toUpperCase()}${words.slice(1)}`;
}

function sourceOf(cik: string, fact: CapexFact): CapexSourceRef {
  return { accession: fact.accession, form: fact.form, filed: fact.filed, url: secFilingIndexUrl(cik, fact.accession) };
}

function uniqueSources(sources: CapexSourceRef[]): CapexSourceRef[] {
  const seen = new Set<string>();
  return sources.filter((s) => (seen.has(s.accession) ? false : (seen.add(s.accession), true)));
}

/* ------------------------------------------------------- fiscal years */

interface Placed {
  fact: CapexFact;
  months: 3 | 6 | 9 | 12;
  startDay: number;
  endDay: number;
}

export interface CapexFiscalYearGroup {
  /** First day of the fiscal year. */
  start: string;
  /** Last day: the 12-month fact's end when filed, else one year after the start minus a day. */
  end: string;
  /** Calendar year of the fiscal year end (Microsoft's year ending 2026-06-30 is FY2026). */
  fiscalYear: number;
  /** Filed ~3-month facts by position in the fiscal year (1..4). */
  quarters: Partial<Record<1 | 2 | 3 | 4, CapexFact>>;
  /** Year-to-date facts: 1 = the first quarter, 2 = six months, 3 = nine months, 4 = twelve months. */
  toDate: Partial<Record<1 | 2 | 3 | 4, CapexFact>>;
}

/**
 * Places every usable fact of a series in its fiscal year. Facts that are neither a ~3-month quarter
 * nor a year-to-date span starting on a fiscal year start (for example trailing-twelve-month rows)
 * are ignored. Problems are pushed to `warnings`. Oldest fiscal year first.
 */
export function groupByFiscalYear(series: CapexSeries, warnings: string[] = []): CapexFiscalYearGroup[] {
  const fye = series.fiscalYearEnd;
  const quarters: Placed[] = [];
  const toDate: Placed[] = [];
  const seen = new Map<string, CapexFact>();

  for (const fact of series.facts) {
    const months = spanMonths(fact.start, fact.end);
    if (months === null) continue;
    const key = `${fact.start}/${fact.end}`;
    const twin = seen.get(key);
    if (twin) {
      if (twin.value !== fact.value) {
        warnings.push(
          `Two facts for ${fact.start} to ${fact.end} disagree (${formatCapexAmount(twin.value, series.currency)} in ${twin.accession}, ${formatCapexAmount(fact.value, series.currency)} in ${fact.accession}); the first is used.`
        );
      }
      continue;
    }
    seen.set(key, fact);
    const placed: Placed = { fact, months, startDay: dayNumber(fact.start), endDay: dayNumber(fact.end) };
    if (months === 3) quarters.push(placed);
    else if (isFiscalYearStart(fact.start, fye)) toDate.push(placed);
  }

  // Fiscal year starts the data shows: year-to-date starts, the day after each 12-month end, and
  // quarters that start a fiscal year.
  const knownStarts = new Set<number>();
  for (const p of toDate) {
    knownStarts.add(p.startDay);
    if (p.months === 12) knownStarts.add(p.endDay + 1);
  }
  for (const q of quarters) if (isFiscalYearStart(q.fact.start, fye)) knownStarts.add(q.startDay);
  const starts = [...knownStarts].sort((a, b) => a - b);

  const fiscalYearStartOf = (q: Placed): number | null => {
    let best: number | null = null;
    for (const s of starts) if (s <= q.startDay && q.endDay - s < 380) best = s;
    if (best !== null) return best;
    // No fact shows this year's start: use the day after the fiscal year end (month-day) before it.
    const [month, day] = fye.split("-").map(Number);
    const startYear = parseIso(q.fact.start).y;
    for (const y of [startYear, startYear - 1]) {
      const candidate = Math.round(Date.UTC(y, month - 1, Math.min(day, daysInMonth(y, month))) / DAY_MS) + 1;
      if (candidate <= q.startDay + FISCAL_YEAR_END_TOLERANCE_DAYS && q.endDay - candidate < 380) return candidate;
    }
    return null;
  };

  const groups = new Map<number, { quarters: CapexFiscalYearGroup["quarters"]; toDate: CapexFiscalYearGroup["toDate"] }>();
  const groupAt = (start: number) => {
    let g = groups.get(start);
    if (!g) {
      g = { quarters: {}, toDate: {} };
      groups.set(start, g);
    }
    return g;
  };

  for (const p of toDate) {
    const g = groupAt(p.startDay);
    const index = (p.months / 3) as 2 | 3 | 4;
    const other = g.toDate[index];
    if (other) {
      warnings.push(`Two ${p.months}-month facts start ${p.fact.start} (to ${other.end} and to ${p.fact.end}); the first is used.`);
      continue;
    }
    g.toDate[index] = p.fact;
  }
  for (const q of quarters) {
    const start = fiscalYearStartOf(q);
    if (start === null) {
      warnings.push(`Cannot place the quarter ${q.fact.start} to ${q.fact.end} in a fiscal year; it is left out.`);
      continue;
    }
    const index = Math.min(4, Math.max(1, Math.round((q.endDay - start + 1) / 91.3))) as 1 | 2 | 3 | 4;
    const expectedStart = start + Math.round((index - 1) * 91.3);
    if (Math.abs(q.startDay - expectedStart) > 10) {
      warnings.push(`The quarter ${q.fact.start} to ${q.fact.end} does not line up with its fiscal year (start ${isoOfDay(start)}); it is left out.`);
      continue;
    }
    const g = groupAt(start);
    const other = g.quarters[index];
    if (other) {
      warnings.push(`Two quarters fall in the same slot (${other.start} to ${other.end} and ${q.fact.start} to ${q.fact.end}); the first is used.`);
      continue;
    }
    g.quarters[index] = q.fact;
    if (index === 1) g.toDate[1] = q.fact;
  }

  return [...groups.entries()]
    .sort(([a], [b]) => a - b)
    .map(([startDay, g]) => {
      const twelve = g.toDate[4];
      const start = isoOfDay(startDay);
      const { y, m, d } = parseIso(start);
      const endDay = twelve ? dayNumber(twelve.end) : Math.round(Date.UTC(y + 1, m - 1, d) / DAY_MS) - 1;
      const end = isoOfDay(endDay);
      return { start, end, fiscalYear: parseIso(end).y, quarters: g.quarters, toDate: g.toDate };
    });
}

/* -------------------------------------------------------------- views */

/** Quarters (direct or derived), fiscal years and warnings for one series. */
export function deriveCapex(series: CapexSeries): CapexView {
  const warnings: string[] = [];
  const groups = groupByFiscalYear(series, warnings);
  const quarters: CapexQuarter[] = [];
  const years: CapexYear[] = [];
  const money = (value: number) => formatCapexAmount(value, series.currency);

  for (const g of groups) {
    for (const index of [1, 2, 3, 4] as const) {
      const label = fiscalLabel({ fiscalYear: g.fiscalYear, fiscalQuarter: index });
      const direct = g.quarters[index];
      const total = g.toDate[index];
      const before = index > 1 ? g.toDate[(index - 1) as 1 | 2 | 3] : undefined;
      // "six months to Jun 30, 2026 ($X) minus three months to Mar 31, 2026 ($Y)"
      const words =
        total && before
          ? `${spanWords((index * 3) as 6 | 9 | 12, total.end)} (${money(total.value)}) minus ${spanWords(((index - 1) * 3) as 3 | 6 | 9, before.end)} (${money(before.value)})`
          : null;

      if (direct) {
        if (words && total && before && total.value - before.value !== direct.value) {
          warnings.push(
            `${label}: the three-month figure (${money(direct.value)}) does not equal ${words}, which is ${money(total.value - before.value)}.`
          );
        }
        quarters.push({
          org: series.org,
          fiscalYear: g.fiscalYear,
          fiscalQuarter: index,
          start: direct.start,
          end: direct.end,
          value: direct.value,
          currency: series.currency,
          derived: false,
          formula: null,
          sources: [sourceOf(series.cik, direct)],
          calendarQuarter: calendarQuarterOf(direct.start, direct.end),
        });
        continue;
      }
      if (!total || !before || !words) continue; // never derive across a gap
      const value = total.value - before.value;
      if (value < 0) {
        warnings.push(`${label}: ${words} is below zero (${money(value)}); the quarter is left out.`);
        continue;
      }
      const start = isoOfDay(dayNumber(before.end) + 1);
      quarters.push({
        org: series.org,
        fiscalYear: g.fiscalYear,
        fiscalQuarter: index,
        start,
        end: total.end,
        value,
        currency: series.currency,
        derived: true,
        formula: `${words[0].toUpperCase()}${words.slice(1)}`,
        sources: uniqueSources([sourceOf(series.cik, total), sourceOf(series.cik, before)]),
        calendarQuarter: calendarQuarterOf(start, total.end),
      });
    }
    const twelve = g.toDate[4];
    if (twelve) {
      years.push({
        org: series.org,
        fiscalYear: g.fiscalYear,
        start: twelve.start,
        end: twelve.end,
        value: twelve.value,
        currency: series.currency,
        source: sourceOf(series.cik, twelve),
      });
    }
  }

  quarters.sort((a, b) => (a.end < b.end ? -1 : a.end > b.end ? 1 : 0));
  years.sort((a, b) => (a.end < b.end ? -1 : a.end > b.end ? 1 : 0));
  return {
    org: series.org,
    currency: series.currency,
    fiscalYearEnd: series.fiscalYearEnd,
    lineItem: series.lineItem ?? null,
    notes: [...series.notes],
    quarters,
    years,
    warnings,
  };
}

/**
 * Every filed quarter or fiscal year-to-date fact, placed in its fiscal year, oldest first. Use it to
 * show a figure that has no quarter yet (for example a six-month figure from a company's first 10-Q).
 */
export function capexPeriods(series: CapexSeries): CapexPeriod[] {
  const out: CapexPeriod[] = [];
  for (const g of groupByFiscalYear(series)) {
    const facts = new Set<CapexFact>([...Object.values(g.quarters), ...Object.values(g.toDate)]);
    for (const fact of facts) {
      const months = spanMonths(fact.start, fact.end)!;
      out.push({
        org: series.org,
        fiscalYear: g.fiscalYear,
        months,
        start: fact.start,
        end: fact.end,
        value: fact.value,
        currency: series.currency,
        label: capexSpanLabel(months, fact.end),
        source: sourceOf(series.cik, fact),
      });
    }
  }
  return out.sort((a, b) => (a.end === b.end ? a.months - b.months : a.end < b.end ? -1 : 1));
}
