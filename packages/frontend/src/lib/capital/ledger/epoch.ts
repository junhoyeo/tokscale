import { z } from "zod";
import { eventId, isoDate, orgId, type CapitalEvent, type Org } from "./schema";

/**
 * Epoch AI review inbox (src/content/capital/inbox/epoch-*.json).
 *
 * Owner decision: Epoch AI data (CC BY 4.0) is imported into an inbox with attribution. Nothing in
 * the inbox is shown on Tokscale pages. A row is published only after an editor re-checks it
 * against a primary source and writes a separate event in src/content/capital/events/. The import
 * (scripts/capital/import-epoch.ts) then marks the matching row "published" with that event id.
 *
 * This module holds the pure, tested steps of the import: CSV parsing, number and date parsing,
 * org mapping, amount proposals, event matching, and the zod schemas of both inbox files.
 */

export const EPOCH_LICENSE = "CC BY 4.0";
export const EPOCH_LICENSE_URL = "https://creativecommons.org/licenses/by/4.0/";
export const EPOCH_INBOX_NOTE =
  "Review inbox. Not shown on Tokscale pages. A row is published only after an editor re-checks it against a primary source.";

/* ---------------------------------------------------------------- numbers */

const DECIMAL = /^-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?$/;

/**
 * One numeric cell as Epoch wrote it. A blank cell is null, never 0. An explicit "0" or "0.0"
 * stays 0 (Epoch uses it for tenders and stock-only deals). Anything that is not a plain decimal
 * throws, so a changed format fails the import instead of becoming a wrong number.
 */
export function parseEpochNumber(text: string | null | undefined): number | null {
  if (text === null || text === undefined) return null;
  const t = text.trim();
  if (t === "") return null;
  if (!DECIMAL.test(t)) throw new Error(`not a number: "${text}"`);
  const n = Number(t);
  if (!Number.isFinite(n)) throw new Error(`not a finite number: "${text}"`);
  return n;
}

/**
 * A decimal cell scaled by 10^digits and rounded to a whole number, without float noise:
 * ("35.836372", 9) -> 35836372000. Used for Epoch's "2025 USD billions" column. Blank -> null.
 */
export function parseEpochScaled(text: string | null | undefined, digits: number): number | null {
  const n = parseEpochNumber(text);
  if (n === null) return null;
  const t = (text as string).trim();
  const m = /^(-?)(\d+)(?:\.(\d+))?$/.exec(t);
  if (!m) return Math.round(n * 10 ** digits);
  const [, sign, whole, fraction = ""] = m;
  let scaled = Number(`${whole}${fraction.padEnd(digits, "0").slice(0, digits)}`);
  if (fraction.length > digits && Number(fraction[digits]) >= 5) scaled += 1;
  return sign === "-" ? -scaled : scaled;
}

/** "2026-01-08" -> "2026-01-08"; blank -> null; any other format throws. */
export function parseEpochDate(text: string | null | undefined): string | null {
  const t = (text ?? "").trim();
  if (t === "") return null;
  if (!/^\d{4}-\d{2}-\d{2}$/.test(t)) throw new Error(`not a YYYY-MM-DD date: "${text}"`);
  return t;
}

const MONTHS: Record<string, number> = {
  jan: 1, january: 1, feb: 2, february: 2, mar: 3, march: 3, apr: 4, april: 4, may: 5, jun: 6, june: 6,
  jul: 7, july: 7, aug: 8, august: 8, sep: 9, sept: 9, september: 9, oct: 10, october: 10, nov: 11,
  november: 11, dec: 12, december: 12,
};

/** The badge on Epoch's data pages: "Updated Oct. 8, 2026" -> "2026-10-08". */
export function parseEpochUpdated(text: string): string {
  const m = /Updated\s+([A-Za-z]+)\.?\s+(\d{1,2}),\s+(\d{4})/.exec(text);
  const month = m ? MONTHS[m[1].toLowerCase()] : undefined;
  if (!m || !month) throw new Error(`no "Updated <Mon>. <d>, <yyyy>" text in: "${text}"`);
  const day = Number(m[2]);
  if (day < 1 || day > 31) throw new Error(`bad day in: "${text}"`);
  return `${m[3]}-${String(month).padStart(2, "0")}-${String(day).padStart(2, "0")}`;
}

/* -------------------------------------------------------------------- CSV */

/** RFC 4180 CSV: quoted fields may hold commas, quotes ("") and line breaks. A BOM is dropped. */
export function parseCsv(text: string): string[][] {
  const rows: string[][] = [];
  let row: string[] = [];
  let field = "";
  let quoted = false;
  let i = text.charCodeAt(0) === 0xfeff ? 1 : 0;
  for (; i < text.length; i++) {
    const c = text[i];
    if (quoted) {
      if (c === '"') {
        if (text[i + 1] === '"') {
          field += '"';
          i++;
        } else quoted = false;
      } else field += c;
    } else if (c === '"' && field === "") quoted = true;
    else if (c === ",") {
      row.push(field);
      field = "";
    } else if (c === "\n" || c === "\r") {
      if (c === "\r" && text[i + 1] === "\n") i++;
      row.push(field);
      rows.push(row);
      row = [];
      field = "";
    } else field += c;
  }
  if (quoted) throw new Error("CSV ends inside a quoted field");
  if (field !== "" || row.length > 0) {
    row.push(field);
    rows.push(row);
  }
  return rows.filter((r) => !(r.length === 1 && r[0] === ""));
}

/** CSV rows as records keyed by the header. A row longer than the header throws. */
export function csvRecords(text: string, requiredColumns: readonly string[] = []): Array<Record<string, string>> {
  const [header, ...rows] = parseCsv(text);
  if (!header) throw new Error("empty CSV");
  if (new Set(header).size !== header.length) throw new Error(`duplicate CSV columns: ${header.join(", ")}`);
  const missing = requiredColumns.filter((c) => !header.includes(c));
  if (missing.length > 0) throw new Error(`CSV is missing columns: ${missing.join(", ")}`);
  return rows.map((cells, n) => {
    if (cells.length > header.length) throw new Error(`CSV row ${n + 2} has ${cells.length} cells; the header has ${header.length}`);
    return Object.fromEntries(header.map((name, j) => [name, cells[j] ?? ""]));
  });
}

/* ------------------------------------------------------------------- URLs */

const httpUrl = (text: string): string | null => {
  const t = text.replace(/\u00a0/g, " ").trim();
  if (!/^https?:\/\/\S+$/i.test(t)) return null;
  try {
    return new URL(t).protocol.startsWith("http") ? t : null;
  } catch {
    return null;
  }
};

/** Links in Epoch's "Selected Sources" markdown ("- [title](url)"), with balanced parentheses in URLs. */
export function markdownLinks(text: string): string[] {
  const urls: string[] = [];
  let at = 0;
  for (;;) {
    const start = text.indexOf("](", at);
    if (start < 0) break;
    let depth = 1;
    let j = start + 2;
    for (; j < text.length && depth > 0; j++) {
      if (text[j] === "(") depth++;
      else if (text[j] === ")") depth--;
      else if (/\s/.test(text[j])) break;
    }
    const url = httpUrl(text.slice(start + 2, depth === 0 ? j - 1 : j));
    if (url) urls.push(url);
    at = j;
  }
  return [...new Set(urls)];
}

/* ------------------------------------------------------------------- orgs */

export interface OrgIndex {
  /** Normalized name -> org id, only for names that belong to exactly one org. */
  byName: ReadonlyMap<string, string>;
}

export const normalizeOrgName = (text: string) => text.normalize("NFKC").trim().replace(/\s+/g, " ").toLowerCase();

/** Index of org names, short names, legal names and aliases. A name two orgs share maps to neither. */
export function buildOrgIndex(orgs: ReadonlyArray<Pick<Org, "id" | "name" | "shortName" | "legalNames" | "aliases">>): OrgIndex {
  const owners = new Map<string, Set<string>>();
  for (const org of orgs) {
    for (const name of [org.name, org.shortName, ...org.legalNames, ...org.aliases]) {
      if (!name) continue;
      const key = normalizeOrgName(name);
      if (!owners.has(key)) owners.set(key, new Set());
      owners.get(key)!.add(org.id);
    }
  }
  const byName = new Map<string, string>();
  for (const [key, ids] of owners) if (ids.size === 1) byName.set(key, [...ids][0]);
  return { byName };
}

/**
 * Our org id for a name as Epoch writes it, or null when it does not map clearly.
 * "Z.ai (Zhipu)" maps when "Z.ai" or "Zhipu" is a known name and both parts agree.
 */
export function mapOrgName(name: string, index: OrgIndex): string | null {
  const direct = index.byName.get(normalizeOrgName(name));
  if (direct) return direct;
  const m = /^(.*?)\s*\(([^()]+)\)\s*$/.exec(name);
  if (!m) return null;
  const ids = new Set([m[1], m[2]].map((part) => index.byName.get(normalizeOrgName(part))).filter((v): v is string => !!v));
  return ids.size === 1 ? [...ids][0] : null;
}

/** Epoch tags names with its confidence: "OpenAI #likely, Microsoft #likely". */
export function splitTaggedNames(text: string): Array<{ name: string; tag: string | null }> {
  return text
    .split(",")
    .map((part) => part.trim())
    .filter(Boolean)
    .map((part) => {
      const m = /^(.*?)\s*#([a-z]+)$/i.exec(part);
      return m ? { name: m[1].trim(), tag: m[2].toLowerCase() } : { name: part, tag: null };
    });
}

/* ---------------------------------------------------------------- schemas */

export const reviewSchema = z
  .object({
    state: z.enum(["unreviewed", "published"]),
    /** The hand-written event that publishes this row. */
    event: eventId.nullable(),
  })
  .strict()
  .refine((r) => (r.state === "published") === (r.event !== null), {
    message: "a published row names its event; an unreviewed row has none",
  });
export type Review = z.infer<typeof reviewSchema>;

export const PROPOSED_MEASURES = ["equity_raised", "debt_raised", "valuation_post"] as const;

export const proposedAmountSchema = z
  .object({
    measure: z.enum(PROPOSED_MEASURES),
    value: z.number().positive(),
    currency: z.literal("USD"),
    basis: z.literal("exact"),
  })
  .strict();
export type ProposedAmount = z.infer<typeof proposedAmountSchema>;

const nullableNumber = z.number().nonnegative().nullable();
const httpUrlSchema = z.string().url().regex(/^https?:\/\//i);

/** Epoch's funding-round CSV columns this import reads. */
export const EPOCH_FUNDING_COLUMNS = [
  "Id",
  "Company",
  "Close date",
  "Status",
  "Funding (equity)",
  "Funding (debt)",
  "Valuation (post-money)",
  "Report date",
  "Source 1",
  "Source 2",
  "Source 3",
  "Type",
  "Confidence",
] as const;

export const epochFundingRowSchema = z
  .object({
    /** Epoch's "Id" ("May 2026 965B Series H"). */
    epochId: z.string().min(1),
    /** Epoch's company name ("Z.ai (Zhipu)"). */
    company: z.string().min(1),
    /** Our org id when the name maps clearly, else null. */
    org: orgId.nullable(),
    closeDate: isoDate.nullable(),
    reportDate: isoDate.nullable(),
    /** Epoch's words: "Closed", "Late discussions", "Cancelled". */
    status: z.string().min(1),
    /** Epoch's words: "Primary", "Secondary", "Other", "Primary,Debt"; null when blank. */
    type: z.string().min(1).nullable(),
    /** Epoch's words: "Confident", "Likely", "Uncertain"; null when blank. */
    confidence: z.string().min(1).nullable(),
    equityUsd: nullableNumber,
    debtUsd: nullableNumber,
    postMoneyUsd: nullableNumber,
    /** The three number cells exactly as Epoch wrote them, so a blank is never mistaken for 0. */
    epochCells: z
      .object({ "Funding (equity)": z.string(), "Funding (debt)": z.string(), "Valuation (post-money)": z.string() })
      .strict(),
    sources: z.array(httpUrlSchema),
    proposed: z.array(proposedAmountSchema),
    review: reviewSchema,
  })
  .strict();
export type EpochFundingRow = z.infer<typeof epochFundingRowSchema>;

/** Epoch's data-center CSV columns this import reads. */
export const EPOCH_DATA_CENTER_COLUMNS = [
  "Name",
  "Owner",
  "Users",
  "Project",
  "Country",
  "Current power (MW)",
  "Current H100 equivalents",
  "Current total capital cost (2025 USD billions)",
  "Selected Sources",
  "Calculations sheet",
] as const;

export const epochDataCenterRowSchema = z
  .object({
    name: z.string().min(1),
    /** As Epoch writes it, with its tag ("Google #confident"). */
    owner: z.string().min(1).nullable(),
    /** As Epoch writes it, with tags ("OpenAI #likely, Microsoft #likely"). */
    users: z.string().min(1).nullable(),
    project: z.string().min(1).nullable(),
    country: z.string().min(1).nullable(),
    currentPowerMw: nullableNumber,
    h100Equivalents: nullableNumber,
    /** Epoch's estimate of the current total capital cost, in 2025 US dollars (whole dollars). */
    estimatedCapitalCost2025Usd: nullableNumber,
    epochCells: z
      .object({
        "Current power (MW)": z.string(),
        "Current H100 equivalents": z.string(),
        "Current total capital cost (2025 USD billions)": z.string(),
      })
      .strict(),
    sources: z.array(httpUrlSchema),
    calculationsSheet: httpUrlSchema.nullable(),
    /** Our org id for the owner when it maps clearly, else null. */
    org: orgId.nullable(),
    review: reviewSchema,
  })
  .strict();
export type EpochDataCenterRow = z.infer<typeof epochDataCenterRowSchema>;

const inboxHeader = {
  /** "Epoch AI, Data on AI companies" */
  dataset: z.string().min(1),
  /** The dataset page. */
  url: httpUrlSchema,
  /** The file the rows came from. */
  downloadUrl: httpUrlSchema,
  license: z.literal(EPOCH_LICENSE),
  licenseUrl: z.literal(EPOCH_LICENSE_URL),
  /** Epoch's license sentence, verbatim. */
  licenseText: z.string().min(1),
  /** The citation Epoch asks for, verbatim (with our access date where Epoch's template has one). */
  attribution: z.string().min(1),
  retrievedAt: isoDate,
  /** Date of the "Updated ..." badge on the dataset page. */
  epochUpdated: isoDate,
  /** The badge text, verbatim ("Updated Oct. 8, 2026"). */
  epochUpdatedText: z.string().regex(/^Updated /),
  /** SHA-256 of the downloaded file (CSV or ZIP). */
  sha256: z.string().regex(/^[0-9a-f]{64}$/),
  note: z.literal(EPOCH_INBOX_NOTE),
};

export const epochFundingInboxSchema = z
  .object({ ...inboxHeader, rows: z.array(epochFundingRowSchema) })
  .strict()
  .superRefine((file, ctx) => {
    const seen = new Set<string>();
    file.rows.forEach((row, i) => {
      if (seen.has(row.epochId)) ctx.addIssue({ code: z.ZodIssueCode.custom, path: ["rows", i, "epochId"], message: `duplicate epochId ${row.epochId}` });
      seen.add(row.epochId);
    });
  });
export type EpochFundingInbox = z.infer<typeof epochFundingInboxSchema>;

export const epochDataCenterInboxSchema = z
  .object({ ...inboxHeader, rows: z.array(epochDataCenterRowSchema) })
  .strict()
  .superRefine((file, ctx) => {
    const seen = new Set<string>();
    file.rows.forEach((row, i) => {
      if (seen.has(row.name)) ctx.addIssue({ code: z.ZodIssueCode.custom, path: ["rows", i, "name"], message: `duplicate name ${row.name}` });
      seen.add(row.name);
    });
  });
export type EpochDataCenterInbox = z.infer<typeof epochDataCenterInboxSchema>;

/* ---------------------------------------------------------------- funding */

type ProposalInput = Pick<EpochFundingRow, "status" | "type" | "equityUsd" | "debtUsd" | "postMoneyUsd">;

const typeParts = (type: string | null) =>
  new Set(
    (type ?? "")
      .split(",")
      .map((t) => t.trim().toLowerCase())
      .filter(Boolean)
  );

/**
 * Ledger amounts an editor could publish for this row, once checked against a primary source.
 *
 * - Only rows Epoch marks "Closed". Cancelled deals and "Late discussions" raised nothing yet,
 *   and the ledger has no "in talks" status.
 * - equity_raised and valuation_post only for "Primary" rounds. A secondary sale (tender) raises
 *   no money for the company, and "Other" rows hold market caps or stock-only mergers, which are
 *   not post-money valuations.
 * - debt_raised whenever Epoch lists debt.
 * - Zero and blank values are never proposed.
 */
export function proposeAmounts(row: ProposalInput): ProposedAmount[] {
  if (row.status.trim().toLowerCase() !== "closed") return [];
  const primary = typeParts(row.type).has("primary");
  const out: ProposedAmount[] = [];
  const add = (measure: ProposedAmount["measure"], value: number | null) => {
    if (value !== null && value > 0) out.push({ measure, value, currency: "USD", basis: "exact" });
  };
  if (primary) add("equity_raised", row.equityUsd);
  add("debt_raised", row.debtUsd);
  if (primary) add("valuation_post", row.postMoneyUsd);
  return out;
}

/** Event amounts that count as the event's equity: raised in a round or invested by one investor. */
const EQUITY_MEASURES = new Set(["equity_raised", "equity_investment"]);
export const MATCH_WINDOW_DAYS = 7;

const dayNumber = (date: string) => Date.UTC(Number(date.slice(0, 4)), Number(date.slice(5, 7)) - 1, Number(date.slice(8, 10))) / 86_400_000;

type MatchInput = Pick<EpochFundingRow, "org" | "closeDate" | "reportDate" | "equityUsd">;
type MatchEvent = Pick<CapitalEvent, "id" | "date" | "dates" | "participants" | "amounts">;

/**
 * Hand-written events that publish this row: the same issuer org, a date within 7 days of
 * Epoch's close date (or its report date when the close date is blank), and the same equity
 * amount in USD. Amounts that are part of a larger amount in the event (partOf) do not count.
 */
export function findMatchingEvents(row: MatchInput, events: ReadonlyArray<MatchEvent>): string[] {
  const date = row.closeDate ?? row.reportDate;
  if (!row.org || !date || row.equityUsd === null || row.equityUsd <= 0) return [];
  const day = dayNumber(date);
  return events
    .filter((event) => {
      const issuer = event.participants.some((p) => p.org === row.org && p.roles.includes("issuer"));
      if (!issuer) return false;
      const dates = [event.date, ...Object.values(event.dates)].filter((d): d is string => typeof d === "string");
      if (!dates.some((d) => Math.abs(dayNumber(d) - day) <= MATCH_WINDOW_DAYS)) return false;
      return event.amounts.some(
        (a) => EQUITY_MEASURES.has(a.measure) && !a.partOf && a.currency === "USD" && a.value !== null && a.value === row.equityUsd
      );
    })
    .map((event) => event.id);
}

/** The one event that publishes this row, or null when none (or more than one) matches. */
export function matchEvent(row: MatchInput, events: ReadonlyArray<MatchEvent>): string | null {
  const ids = findMatchingEvents(row, events);
  return ids.length === 1 ? ids[0] : null;
}

export const reviewFor = (event: string | null): Review =>
  event ? { state: "published", event } : { state: "unreviewed", event: null };

/** One CSV record of Epoch's funding rounds -> an inbox row (review left unreviewed). */
export function normalizeFundingRow(record: Record<string, string>, index: OrgIndex): { row: EpochFundingRow; warnings: string[] } {
  const warnings: string[] = [];
  const id = record.Id.trim();
  const sources: string[] = [];
  for (const column of ["Source 1", "Source 2", "Source 3"]) {
    const raw = (record[column] ?? "").trim();
    if (!raw) continue;
    const url = httpUrl(raw);
    if (url) sources.push(url);
    else warnings.push(`${id}: ${column} is not a URL: "${raw}"`);
  }
  const blankToNull = (text: string | undefined) => (text ?? "").trim() || null;
  const company = record.Company.trim();
  const base = {
    epochId: id,
    company,
    org: mapOrgName(company, index),
    closeDate: parseEpochDate(record["Close date"]),
    reportDate: parseEpochDate(record["Report date"]),
    status: record.Status.trim(),
    type: blankToNull(record.Type),
    confidence: blankToNull(record.Confidence),
    equityUsd: parseEpochNumber(record["Funding (equity)"]),
    debtUsd: parseEpochNumber(record["Funding (debt)"]),
    postMoneyUsd: parseEpochNumber(record["Valuation (post-money)"]),
    epochCells: {
      "Funding (equity)": record["Funding (equity)"] ?? "",
      "Funding (debt)": record["Funding (debt)"] ?? "",
      "Valuation (post-money)": record["Valuation (post-money)"] ?? "",
    },
    sources: [...new Set(sources)],
  };
  return { row: { ...base, proposed: proposeAmounts(base), review: reviewFor(null) }, warnings };
}

/** One CSV record of Epoch's data centers -> an inbox row (review left unreviewed). */
export function normalizeDataCenterRow(record: Record<string, string>, index: OrgIndex): { row: EpochDataCenterRow; warnings: string[] } {
  const warnings: string[] = [];
  const blankToNull = (text: string | undefined) => (text ?? "").trim() || null;
  const name = record.Name.trim();
  const owner = blankToNull(record.Owner);
  const owners = owner ? splitTaggedNames(owner) : [];
  const ownerIds = new Set(owners.map((o) => mapOrgName(o.name, index)));
  const org = owners.length > 0 && ownerIds.size === 1 && !ownerIds.has(null) ? [...ownerIds][0] : null;
  const sheet = blankToNull(record["Calculations sheet"]);
  const calculationsSheet = sheet ? httpUrl(sheet) : null;
  if (sheet && !calculationsSheet) warnings.push(`${name}: calculations sheet is not a URL: "${sheet}"`);
  const row: EpochDataCenterRow = {
    name,
    owner,
    users: blankToNull(record.Users),
    project: blankToNull(record.Project),
    country: blankToNull(record.Country),
    currentPowerMw: parseEpochNumber(record["Current power (MW)"]),
    h100Equivalents: parseEpochNumber(record["Current H100 equivalents"]),
    estimatedCapitalCost2025Usd: parseEpochScaled(record["Current total capital cost (2025 USD billions)"], 9),
    epochCells: {
      "Current power (MW)": record["Current power (MW)"] ?? "",
      "Current H100 equivalents": record["Current H100 equivalents"] ?? "",
      "Current total capital cost (2025 USD billions)": record["Current total capital cost (2025 USD billions)"] ?? "",
    },
    sources: markdownLinks(record["Selected Sources"] ?? ""),
    calculationsSheet,
    org,
    review: reviewFor(null),
  };
  return { row, warnings };
}
