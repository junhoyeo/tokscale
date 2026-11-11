/**
 * Build src/content/capital/metrics/capex.json ("cash paid for property and equipment") from SEC XBRL.
 *
 * Source: SEC XBRL companyfacts (https://data.sec.gov/api/xbrl/companyfacts/CIK##########.json), the SEC
 * submissions record (fiscal year end, latest reports) and, for the company's own line label, the
 * cash flow statement page (R file) of two filings. Every response is cached under
 * .stealth-cache/capital/sec/ (see lib/sec-client.ts), so reruns read from disk.
 *
 * Rules (owner decisions, docs/stealth/capital-v2-plan.md 3.2 and 4.10):
 * - One XBRL concept per company (fallbacks only fill periods the main concept does not cover).
 * - Original currency only. Facts from a filing that reports this line mainly in another currency
 *   (convenience translations) are not used.
 * - Duration facts only: ~3-month quarters (80-100 days) and year-to-date spans of 6, 9 or 12 months
 *   that start on a fiscal year start. Fiscal year starts come from 12-month facts in annual reports
 *   (start, and the day after the end); when no annual report covers a date (new filers), the day
 *   after the fiscal year end in the SEC submissions record. Trailing-twelve-month rows drop out.
 * - Forms 10-Q, 10-K, 10-KT, 20-F, 40-F and their amendments (6-K only where a rule allows it).
 *   For each period the earliest filed fact is kept (the original report); later revisions are noted.
 * - The last 4 fiscal years plus the current one.
 * - A company is skipped (with a reason) rather than shipped with a wrong or stale series.
 *
 * Usage (from packages/frontend):
 *   bun scripts/capital/build-capex.ts             # use cached SEC responses, fetch only what is missing
 *   bun scripts/capital/build-capex.ts --refresh   # fetch every SEC response again (new filings)
 *   bun scripts/capital/build-capex.ts --replay    # offline: cached responses only, fail if one is missing
 *   bun scripts/capital/build-capex.ts --check     # build in memory; exit 1 if capex.json would change
 *   bun scripts/capital/build-capex.ts --dry-run   # build and print the report, do not write
 */
import { existsSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import {
  capexSpanLabel,
  deriveCapex,
  fiscalLabel,
  formatCapexAmount,
  groupByFiscalYear,
  isFiscalYearStart,
  spanMonths,
} from "../../src/lib/capital/ledger/capex";
import { capexFileSchema, type CapexFact, type CapexFile, type CapexSeries } from "../../src/lib/capital/ledger/schema";
import { SecClient, SecHttpError, companyFactsUrl, padCik, submissionsUrl, type SecMode } from "./lib/sec-client";

const OUTPUT = fileURLToPath(new URL("../../src/content/capital/metrics/capex.json", import.meta.url));
const ORGS_FILE = fileURLToPath(new URL("../../src/content/capital/orgs.json", import.meta.url));

const DEFINITION =
  "Cash paid for property and equipment (purchases of property, plant and equipment) as tagged in SEC XBRL filings. Excludes finance leases and assets acquired with debt. Not capex guidance.";

/** Fiscal years kept: the latest one with data plus the 4 before it. */
const FISCAL_YEARS_BACK = 4;
/** A series whose latest figure ends more than this many days before the fetch is stale and skipped. */
const STALE_AFTER_DAYS = 730;

interface CompanyRule {
  org: string;
  /** Concepts in priority order. The first is the company's line; later ones only fill periods the first does not cover. */
  concepts: string[];
  /** ISO 4217 unit to read; the company's reporting currency for this line. */
  currency: string;
  /** Allow facts from 6-K filings (only for foreign filers that tag interim results there). */
  allow6K?: boolean;
  /** Caveats checked by hand against the XBRL facts. */
  notes?: string[];
}

const PPE = "us-gaap:PaymentsToAcquirePropertyPlantAndEquipment";
const PRODUCTIVE = "us-gaap:PaymentsToAcquireProductiveAssets";

/** Concept map, read from each company's companyfacts on 2026-10-21. */
const RULES: CompanyRule[] = [
  { org: "org_microsoft", concepts: [PPE], currency: "USD" },
  // Amazon: PaymentsToAcquireProductiveAssets in recent filings; PaymentsToAcquirePropertyPlantAndEquipment for periods up to 2017-03-31.
  { org: "org_amazon", concepts: [PRODUCTIVE, PPE], currency: "USD" },
  { org: "org_alphabet", concepts: [PPE], currency: "USD" },
  { org: "org_meta", concepts: [PPE], currency: "USD" },
  { org: "org_oracle", concepts: [PPE], currency: "USD" },
  { org: "org_coreweave", concepts: [PPE], currency: "USD" },
  // Nebius tags only its annual 20-F; older 20-F reports (as Yandex N.V., its former name) gave this line in RUB with a USD translation.
  { org: "org_nebius", concepts: [PPE], currency: "USD" },
  { org: "org_spacex", concepts: [PPE], currency: "USD" },
  { org: "org_nvidia", concepts: [PRODUCTIVE, PPE], currency: "USD" },
  { org: "org_amd", concepts: [PPE], currency: "USD" },
  { org: "org_broadcom", concepts: [PPE], currency: "USD" },
  { org: "org_akamai", concepts: [PPE], currency: "USD" },
  { org: "org_cerebras", concepts: [PPE], currency: "USD" },
  { org: "org_terawulf", concepts: [PPE], currency: "USD" },
  { org: "org_vistra", concepts: [PPE, PRODUCTIVE], currency: "USD" },
  { org: "org_constellation_energy", concepts: [PPE], currency: "USD" },
  {
    org: "org_talen_energy",
    concepts: [PPE, PRODUCTIVE],
    currency: "USD",
    notes: ["2023 is reported as two periods (Jan 1 to May 17 and May 18 to Dec 31), so there are no 2023 figures here."],
  },
  { org: "org_x_energy", concepts: [PRODUCTIVE, PPE], currency: "USD" },
  // TSMC reports under IFRS in New Taiwan dollars; the USD figures in its 20-F are convenience translations.
  { org: "org_tsmc", concepts: ["ifrs-full:PurchaseOfPropertyPlantAndEquipmentClassifiedAsInvestingActivities"], currency: "TWD" },
  // Alibaba tagged this line until the period ending 2020-09-30; the staleness rule skips it.
  { org: "org_alibaba", concepts: ["us-gaap:PaymentsToAcquireOtherPropertyPlantAndEquipment", PPE], currency: "CNY", allow6K: true },
];

const ANNUAL_FORMS = new Set(["10-K", "10-K/A", "10-KT", "10-KT/A", "20-F", "20-F/A", "40-F", "40-F/A"]);
const PERIODIC_FORMS = new Set(["10-Q", "10-Q/A", ...ANNUAL_FORMS]);

/* ------------------------------------------------------------- types */

interface XbrlFact {
  start?: string;
  end: string;
  val: number;
  accn: string;
  fy?: number | null;
  fp?: string | null;
  form: string;
  filed: string;
  frame?: string;
}

interface CompanyFacts {
  cik: number;
  entityName: string;
  facts: Record<string, Record<string, { label?: string | null; units: Record<string, XbrlFact[]> }>>;
}

interface Submissions {
  name: string;
  fiscalYearEnd?: string | null;
  filings: { recent: { accessionNumber: string[]; form: string[]; filingDate: string[]; reportDate: string[] } };
}

interface Candidate extends XbrlFact {
  start: string;
  concept: string;
  priority: number;
}

interface Built {
  org: string;
  name: string;
  series?: CapexSeries;
  skip?: string;
  log: string[];
  fetchedAt: string[];
}

/* ------------------------------------------------------------ helpers */

const DAY_MS = 86_400_000;
const dayNumber = (iso: string) => Math.round(Date.parse(`${iso}T00:00:00Z`) / DAY_MS);
const byText = (a: string, b: string) => (a < b ? -1 : a > b ? 1 : 0);
/** Same month and day, `years` later (Feb 29 becomes Feb 28 in a common year). */
function addYears(iso: string, years: number): string {
  const [y, m, d] = iso.split("-").map(Number);
  const lastDay = new Date(Date.UTC(y + years, m, 0)).getUTCDate();
  return new Date(Date.UTC(y + years, m - 1, Math.min(d, lastDay))).toISOString().slice(0, 10);
}
const isAmendment = (form: string) => form.endsWith("/A");

function decodeEntities(text: string): string {
  return text
    .replace(/&#(\d+);/g, (_, n: string) => String.fromCodePoint(Number(n)))
    .replace(/&#x([0-9a-f]+);/gi, (_, n: string) => String.fromCodePoint(parseInt(n, 16)))
    .replace(/&nbsp;/g, " ")
    .replace(/&quot;/g, '"')
    .replace(/&apos;/g, "'")
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">")
    .replace(/&amp;/g, "&")
    .replace(/\s+/g, " ")
    .trim();
}

/**
 * Largest power of ten (up to 10^6) that divides a non-zero value: the precision it was reported in.
 * Filings report in units, thousands or millions; more trailing zeros than that are a coincidence.
 */
function precisionOf(value: number): number {
  let p = 1;
  while (p < 1e6 && value % (p * 10) === 0) p *= 10;
  return p;
}

/** True when two reported values differ by more than rounding (1,407,359,000 vs 1,407,000,000 is rounding). */
function materiallyDifferent(a: number, b: number): boolean {
  if (a === b) return false;
  if (a === 0 || b === 0) return true;
  const coarse = Math.max(precisionOf(a), precisionOf(b));
  return Math.round(a / coarse) !== Math.round(b / coarse);
}

/**
 * The filing's main currency for a concept when it is not `currency` (the `currency` figures in that
 * filing are then a convenience translation, like Yandex's USD column next to RUB); otherwise null.
 */
function translationFiling(units: Record<string, XbrlFact[]>, accession: string, currency: string): string | null {
  const counts = new Map<string, number>();
  for (const [unit, list] of Object.entries(units)) {
    if (!/^[A-Z]{3}$/.test(unit)) continue;
    for (const f of list) if (f.accn === accession) counts.set(unit, (counts.get(unit) ?? 0) + 1);
  }
  const mine = counts.get(currency) ?? 0;
  const main = [...counts.entries()].sort((a, b) => b[1] - a[1])[0];
  return main && main[0] !== currency && main[1] > mine ? main[0] : null;
}

/** "Property, plant and equipment expenditures (Note 7)" -> "Property, plant and equipment expenditures". */
function cleanLabel(label: string): string {
  let out = label.trim();
  for (let previous = ""; previous !== out; ) {
    previous = out;
    out = out.replace(/\s*\([^()]*\)\s*$/, "").trim();
  }
  return out || label.trim();
}

function folderUrl(cik: string, accession: string): string {
  return `https://www.sec.gov/Archives/edgar/data/${Number(cik)}/${accession.replace(/-/g, "")}`;
}

/** The company's own label for a concept, read from the cash flow statement page of one filing. */
async function lineLabel(sec: SecClient, cik: string, accession: string, concept: string, log: string[]): Promise<string | null> {
  // Fetch errors are not caught: a build that cannot read a page fails instead of silently dropping the label.
  const folder = folderUrl(cik, accession);
  const summaryRes = await sec.get(`${folder}/FilingSummary.xml`);
  if (summaryRes.status !== 200) {
    log.push(`label: no FilingSummary.xml in ${accession} (HTTP ${summaryRes.status})`);
    return null;
  }
  const summary = summaryRes.body;
  const reports = [...summary.matchAll(/<Report\b[^>]*>([\s\S]*?)<\/Report>/g)].map((m) => {
    const field = (name: string) => decodeEntities(new RegExp(`<${name}>([\\s\\S]*?)</${name}>`).exec(m[1])?.[1] ?? "");
    return { file: field("HtmlFileName"), shortName: field("ShortName"), longName: field("LongName"), category: field("MenuCategory") };
  });
  const cashFlow = reports.filter(
    (r) =>
      r.file &&
      /cash\s*flows?/i.test(r.shortName) &&
      !/parenthetical|details?|tables?|policies|supplemental/i.test(r.shortName) &&
      (r.category === "Statements" || / - Statement - /i.test(r.longName))
  );
  if (cashFlow.length === 0) {
    log.push(`label: no cash flow statement page listed in ${accession}`);
    return null;
  }
  const tag = concept.replace(":", "_").replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  for (const report of cashFlow) {
    const res = await sec.get(`${folder}/${report.file}`);
    if (res.status !== 200) continue;
    const match = new RegExp(`defref_${tag}'[^>]*>([^<]+)</a>`).exec(res.body);
    if (match) return cleanLabel(decodeEntities(match[1]));
  }
  log.push(`label: ${concept} not found on the cash flow page(s) of ${accession}`);
  return null;
}

/* ---------------------------------------------------------- one series */

async function buildCompany(sec: SecClient, rule: CompanyRule, cik: string, orgName: string): Promise<Built> {
  const log: string[] = [];
  const fetchedAt: string[] = [];
  const out = (fields: Partial<Built>): Built => ({ org: rule.org, name: orgName, log, fetchedAt, ...fields });

  // 1. Submissions: fiscal year end and the latest periodic report.
  const subs = await sec.getJson<Submissions>(submissionsUrl(cik));
  fetchedAt.push(subs.fetchedAt);
  const recent = subs.data.filings.recent;
  const reports = recent.form
    .map((form, i) => ({ form, filed: recent.filingDate[i], period: recent.reportDate[i], accession: recent.accessionNumber[i] }))
    .filter((r) => PERIODIC_FORMS.has(r.form) && !isAmendment(r.form) && r.period);
  let fiscalYearEnd: string | null = null;
  if (subs.data.fiscalYearEnd && /^\d{4}$/.test(subs.data.fiscalYearEnd)) {
    fiscalYearEnd = `${subs.data.fiscalYearEnd.slice(0, 2)}-${subs.data.fiscalYearEnd.slice(2)}`;
  } else {
    const annual = reports.filter((r) => ANNUAL_FORMS.has(r.form)).sort((a, b) => byText(b.filed, a.filed))[0];
    if (annual) {
      fiscalYearEnd = annual.period.slice(5);
      log.push(`fiscal year end: the submissions record has none; using the period of the latest ${annual.form} (${annual.period})`);
    }
  }
  if (!fiscalYearEnd) return out({ skip: "the SEC submissions record has no fiscal year end and no annual report" });

  // 2. Company facts.
  let facts: CompanyFacts;
  try {
    const res = await sec.getJson<CompanyFacts>(companyFactsUrl(cik));
    facts = res.data;
    fetchedAt.push(res.fetchedAt);
  } catch (error) {
    if (error instanceof SecHttpError && error.status === 404) return out({ skip: "SEC has no XBRL company facts for this CIK" });
    throw error;
  }

  const allowed = (form: string) => {
    const base = form.replace(/\/A$/, "");
    if (base === "6-K") return Boolean(rule.allow6K);
    return ["10-Q", "10-K", "10-KT", "20-F", "40-F"].includes(base);
  };

  // 3. Candidates in the rule's currency, minus convenience translations.
  const candidates: Candidate[] = [];
  const otherCurrencies = new Set<string>();
  const translatedPeriods = new Set<string>();
  let translated = 0;
  rule.concepts.forEach((concept, priority) => {
    const [taxonomy, name] = concept.split(":");
    const units = facts.facts[taxonomy]?.[name]?.units;
    if (!units) return;
    for (const f of units[rule.currency] ?? []) {
      if (!f.start) continue;
      const other = translationFiling(units, f.accn, rule.currency);
      if (other) {
        translated++;
        otherCurrencies.add(other);
        translatedPeriods.add(`${f.start}/${f.end}`);
        continue;
      }
      if (!allowed(f.form)) continue;
      if (f.val < 0) {
        log.push(`dropped a negative fact ${f.start}..${f.end} ${f.val} in ${f.accn}`);
        continue;
      }
      candidates.push({ ...f, start: f.start, concept, priority });
    }
  });
  if (translated > 0) log.push(`dropped ${translated} ${rule.currency} facts that translate a filing's ${[...otherCurrencies].join("/")} figures`);
  if (candidates.length === 0) return out({ skip: `no ${rule.currency} facts for ${rule.concepts.join(" or ")}` });

  // 4. Fiscal year starts and the span filter.
  const annualStarts = new Set<number>();
  for (const f of candidates) {
    if (ANNUAL_FORMS.has(f.form) && spanMonths(f.start, f.end) === 12) {
      annualStarts.add(dayNumber(f.start));
      annualStarts.add(dayNumber(f.end) + 1);
    }
  }
  const nearAnnualStart = (day: number) => [...annualStarts].some((s) => Math.abs(s - day) <= 20);
  const isYearStart = (date: string) => {
    const day = dayNumber(date);
    if (annualStarts.has(day)) return true;
    return !nearAnnualStart(day) && isFiscalYearStart(date, fiscalYearEnd!);
  };
  const oddSpans: string[] = [];
  const usable = candidates.filter((f) => {
    const months = spanMonths(f.start, f.end);
    if (months === 3) return true;
    if (months !== null && isYearStart(f.start)) return true;
    oddSpans.push(`${f.start}..${f.end}`);
    return false;
  });

  // 5. One fact per period: the main concept first, then the earliest filed (the original report).
  const byPeriod = new Map<string, Candidate[]>();
  for (const f of usable) {
    const key = `${f.start}/${f.end}`;
    byPeriod.set(key, [...(byPeriod.get(key) ?? []), f]);
  }
  const chosen: Candidate[] = [];
  const revisions: { first: Candidate; later: Candidate }[] = [];
  for (const group of byPeriod.values()) {
    group.sort(
      (a, b) =>
        a.priority - b.priority ||
        byText(a.filed, b.filed) ||
        Number(isAmendment(a.form)) - Number(isAmendment(b.form)) ||
        byText(a.accn, b.accn)
    );
    const first = group[0];
    chosen.push(first);
    const later = group
      .filter((f) => f.concept === first.concept && f.filed > first.filed && materiallyDifferent(f.val, first.val))
      .sort((a, b) => byText(a.filed, b.filed))[0];
    if (later) revisions.push({ first, later });
  }

  // 6. Keep the latest fiscal year with data plus the 4 before it.
  const toFact = (f: Candidate): CapexFact => ({
    concept: f.concept,
    start: f.start,
    end: f.end,
    value: f.val,
    form: f.form,
    accession: f.accn,
    filed: f.filed,
    ...(typeof f.fy === "number" ? { fy: f.fy } : {}),
    ...(typeof f.fp === "string" && f.fp ? { fp: f.fp } : {}),
  });
  const draft: CapexSeries = {
    org: rule.org,
    cik,
    currency: rule.currency,
    fiscalYearEnd,
    facts: chosen.map(toFact).sort((a, b) => byText(a.start, b.start) || byText(a.end, b.end)),
    notes: [],
  };
  const groups = groupByFiscalYear(draft);
  if (groups.length === 0) return out({ skip: "no quarter or fiscal year-to-date facts" });
  // 10 days of slack so a 52/53-week year that starts a few days later still counts.
  const cutoff = dayNumber(addYears(groups[groups.length - 1].start, -FISCAL_YEARS_BACK)) - 10;
  const kept = new Set<CapexFact>();
  for (const g of groups) {
    if (dayNumber(g.start) < cutoff) continue;
    for (const f of [...Object.values(g.quarters), ...Object.values(g.toDate)]) if (f) kept.add(f);
  }
  const keptFacts = [...kept].sort((a, b) => byText(a.start, b.start) || byText(a.end, b.end));
  const windowStart = keptFacts[0].start;
  const latestEnd = keptFacts.reduce((m, f) => (f.end > m ? f.end : m), "");
  const newestFetch = fetchedAt.reduce((m, t) => (t > m ? t : m), "");
  if (dayNumber(newestFetch.slice(0, 10)) - dayNumber(latestEnd) > STALE_AFTER_DAYS) {
    return out({ skip: `stale: the latest ${rule.concepts.join(" / ")} fact ends ${latestEnd}` });
  }
  for (const span of new Set(oddSpans)) if (span.slice(0, 10) >= windowStart) log.push(`not a quarter or fiscal year-to-date span: ${span}`);

  // 7. Notes (plain words; each one checked against the data above).
  const notes: string[] = [...(rule.notes ?? [])];
  const usedConcepts = [...new Set(keptFacts.map((f) => f.concept))];
  if (usedConcepts.length > 1) {
    const main = rule.concepts[0];
    const firstMain = keptFacts.find((f) => f.concept === main);
    notes.push(
      `The XBRL tag changed: figures before ${firstMain ? firstMain.start : "the latest period"} use ${usedConcepts.filter((c) => c !== main).join(", ")}; later figures use ${main}.`
    );
  }
  const keptKeys = new Set(keptFacts.map((f) => `${f.accession}|${f.start}|${f.end}`));
  const keptRevisions = revisions.filter((r) => keptKeys.has(`${r.first.accn}|${r.first.start}|${r.first.end}`));
  if (keptRevisions.length > 0) {
    const change = (r: { first: Candidate; later: Candidate }) => Math.abs(r.later.val - r.first.val) / Math.max(r.first.val, 1);
    const example = [...keptRevisions].sort((a, b) => change(b) - change(a))[0];
    const span = capexSpanLabel(spanMonths(example.first.start, example.first.end)!, example.first.end);
    notes.push(
      `Later filings revised ${keptRevisions.length === 1 ? "one of these figures" : `${keptRevisions.length} of these figures`} ` +
        `(for example, ${span[0].toLowerCase()}${span.slice(1)}: ${formatCapexAmount(example.first.val, rule.currency)} first reported, ` +
        `${formatCapexAmount(example.later.val, rule.currency)} in the ${example.later.form} filed ${example.later.filed}). This data keeps the first reported figures.`
    );
  }
  if (translated > 0) {
    const replaced = [...new Set(keptFacts.filter((f) => translatedPeriods.has(`${f.start}/${f.end}`)).map((f) => f.end.slice(0, 4)))];
    notes.push(
      `Older filings report this line in ${[...otherCurrencies].join(" and ")} with a ${rule.currency} translation; they are not used.` +
        (replaced.length > 0
          ? ` Figures for ${replaced.join(", ").replace(/, ([^,]*)$/, " and $1")} come from later filings that report in ${rule.currency}.`
          : "")
    );
  }
  if (keptFacts.every((f) => spanMonths(f.start, f.end) === 12)) {
    notes.push("This series has full-year figures only (from annual reports), so there are no quarters.");
  }
  // Finance leases: only when recent filings in this currency (not translations) tag the principal payments.
  const financeUnits = facts.facts["us-gaap"]?.FinanceLeasePrincipalPayments?.units ?? {};
  const recentFinance = (financeUnits[rule.currency] ?? []).filter(
    (f) =>
      f.start &&
      !translationFiling(financeUnits, f.accn, rule.currency) &&
      dayNumber(latestEnd) - dayNumber(f.end) <= STALE_AFTER_DAYS
  );
  if (recentFinance.length > 0) {
    notes.push(`Excludes finance leases: ${orgName} tags finance lease principal payments separately (FinanceLeasePrincipalPayments).`);
  }
  const latestReport = reports.sort((a, b) => byText(b.period, a.period) || byText(b.filed, a.filed))[0];
  if (latestReport && latestReport.period > latestEnd) {
    notes.push(
      `The SEC XBRL data has no figure for this line from the latest ${latestReport.form} (filed ${latestReport.filed}, period ending ${latestReport.period}).`
    );
  }

  // 8. The company's own line label: newest filing used, compared with the oldest.
  const byFiled = [...keptFacts].sort((a, b) => byText(a.filed, b.filed) || byText(a.accession, b.accession));
  const newest = byFiled[byFiled.length - 1];
  const oldest = byFiled[0];
  const lineItem = await lineLabel(sec, cik, newest.accession, newest.concept, log);
  if (lineItem && oldest.accession !== newest.accession) {
    const older = await lineLabel(sec, cik, oldest.accession, oldest.concept, log);
    if (older && older.toLowerCase() !== lineItem.toLowerCase()) {
      notes.push(`The oldest filing used here (${oldest.form} filed ${oldest.filed}) labels this line "${older}".`);
    }
  }
  if (lineItem && /intangible/i.test(lineItem)) {
    notes.push("This line also includes purchases of intangible assets.");
  }

  return out({
    series: {
      org: rule.org,
      cik,
      currency: rule.currency,
      fiscalYearEnd,
      ...(lineItem ? { lineItem } : {}),
      facts: keptFacts,
      notes,
    },
  });
}

/* ---------------------------------------------------------------- main */

function parseArgs(argv: string[]): { mode: SecMode; write: boolean; check: boolean } {
  const known = new Set(["--replay", "--refresh", "--check", "--dry-run", "--help"]);
  for (const arg of argv) if (!known.has(arg)) throw new Error(`unknown argument ${arg} (try --help)`);
  if (argv.includes("--help")) {
    console.log("bun scripts/capital/build-capex.ts [--refresh | --replay] [--check | --dry-run]");
    process.exit(0);
  }
  if (argv.includes("--replay") && argv.includes("--refresh")) throw new Error("--replay and --refresh do not mix");
  const mode: SecMode = argv.includes("--replay") ? "replay" : argv.includes("--refresh") ? "refresh" : "cache";
  const check = argv.includes("--check");
  return { mode, check, write: !check && !argv.includes("--dry-run") };
}

async function main(): Promise<void> {
  const { mode, write, check } = parseArgs(process.argv.slice(2));
  const sec = new SecClient({ mode, log: (line) => console.error(`  ${line}`) });
  const orgs = JSON.parse(readFileSync(ORGS_FILE, "utf8")) as { orgs: { id: string; name: string; ids?: { secCik?: string[] } }[] };
  const orgById = new Map(orgs.orgs.map((o) => [o.id, o]));

  const built = await Promise.all(
    RULES.map(async (rule): Promise<Built> => {
      const org = orgById.get(rule.org);
      const cik = org?.ids?.secCik?.[0];
      if (!org || !cik) return { org: rule.org, name: rule.org, skip: "no SEC CIK in orgs.json", log: [], fetchedAt: [] };
      return buildCompany(sec, rule, padCik(cik), org.name);
    })
  );

  const series = built.flatMap((b) => (b.series ? [b.series] : [])).sort((a, b) => byText(a.org, b.org));
  const fetched = built.filter((b) => b.series).flatMap((b) => b.fetchedAt);
  const retrievedAt = fetched.reduce((m, t) => (t < m ? t : m), fetched[0] ?? new Date().toISOString());
  const file: CapexFile = capexFileSchema.parse({ retrievedAt, source: "SEC XBRL companyfacts", definition: DEFINITION, series });
  const text = `${JSON.stringify(file, null, 2)}\n`;

  // Report.
  console.log(`SEC requests: ${sec.stats.network} network (${sec.stats.retries} retries), ${sec.stats.cache} from cache; mode ${mode}`);
  console.log(`retrievedAt ${retrievedAt}`);
  for (const b of built) {
    if (!b.series) {
      console.log(`SKIP ${b.org}: ${b.skip}`);
      for (const line of b.log) console.log(`     ${line}`);
      continue;
    }
    const view = deriveCapex(b.series);
    const last = view.quarters[view.quarters.length - 1];
    const year = view.years[view.years.length - 1];
    const money = (v: number) => formatCapexAmount(v, view.currency);
    console.log(
      `OK   ${b.org} ${b.series.facts.length} facts, ${view.quarters.length} quarters (${view.quarters.filter((q) => q.derived).length} derived), ${view.years.length} years; ` +
        `latest quarter ${last ? `${fiscalLabel(last)} ${last.start}..${last.end} ${money(last.value)}${last.derived ? " derived" : ""}` : "none"}; ` +
        `latest year ${year ? `FY${year.fiscalYear} ${money(year.value)}` : "none"}`
    );
    console.log(`     line item: ${b.series.lineItem ?? "(not found)"}`);
    for (const note of b.series.notes) console.log(`     note: ${note}`);
    for (const warning of view.warnings) console.log(`     WARNING: ${warning}`);
    for (const line of b.log) console.log(`     ${line}`);
  }

  if (check) {
    const current = existsSync(OUTPUT) ? readFileSync(OUTPUT, "utf8") : "";
    if (current !== text) {
      console.log(`capex.json is out of date (run without --check to rewrite it)`);
      process.exit(1);
    }
    console.log("capex.json is up to date");
    return;
  }
  if (write) {
    const tmp = `${OUTPUT}.${process.pid}.tmp`;
    writeFileSync(tmp, text);
    renameSync(tmp, OUTPUT);
    console.log(`wrote ${OUTPUT} (${series.length} series, ${series.reduce((n, s) => n + s.facts.length, 0)} facts, ${text.length} bytes)`);
  }
}

if (import.meta.main) {
  try {
    await main();
  } catch (error) {
    console.error(`build-capex failed: ${error instanceof Error ? error.message : String(error)}`);
    process.exit(1);
  }
}
