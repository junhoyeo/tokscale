/**
 * Import Epoch AI data (CC BY 4.0) into the Capital review inbox.
 *
 *   bun scripts/capital/import-epoch.ts                    # newest cached snapshot; fetch only if there is none
 *   bun scripts/capital/import-epoch.ts --refresh          # take a new snapshot dated today (UTC)
 *   bun scripts/capital/import-epoch.ts --date 2026-10-21  # use (or complete) that snapshot
 *   bun scripts/capital/import-epoch.ts --replay           # offline; a missing file is an error
 *
 * Writes src/content/capital/inbox/epoch-funding-rounds.json and epoch-data-centers.json.
 * Nothing in the inbox is shown on Tokscale pages. Rerun after adding events: a row is marked
 * "published" when a hand-written event in src/content/capital/events/ has the same issuer org,
 * a date within 7 days and the same equity amount.
 *
 * Raw downloads (dataset pages, the funding CSV, the data-center ZIP) are cached under
 * .stealth-cache/capital/epoch/<retrieval date>/ with a manifest (url, fetchedAt, status, sha256).
 * They are gitignored and never copied elsewhere.
 */
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, readdirSync, renameSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { inflateRawSync } from "node:zlib";
import {
  EPOCH_DATA_CENTER_COLUMNS,
  EPOCH_FUNDING_COLUMNS,
  EPOCH_INBOX_NOTE,
  EPOCH_LICENSE,
  EPOCH_LICENSE_URL,
  buildOrgIndex,
  csvRecords,
  epochDataCenterInboxSchema,
  epochFundingInboxSchema,
  findMatchingEvents,
  normalizeDataCenterRow,
  normalizeFundingRow,
  parseEpochUpdated,
  reviewFor,
  type EpochDataCenterInbox,
  type EpochFundingInbox,
} from "@/lib/capital/ledger/epoch";
import { eventFileSchema, orgsFileSchema, type CapitalEvent, type Org } from "@/lib/capital/ledger/schema";

const ROOT = join(import.meta.dir, "..", "..");
const CONTENT = join(ROOT, "src", "content", "capital");
const INBOX = join(CONTENT, "inbox");
const CACHE_ROOT = join(ROOT, ".stealth-cache", "capital", "epoch");
const USER_AGENT = "TokscaleResearch/1.0 (i@junho.io)";
const TIMEOUT_MS = 30_000;
const MAX_ATTEMPTS = 4;
const MIN_INTERVAL_MS = 1000;

const COMPANIES_PAGE = "https://epoch.ai/data/ai-companies";
const DATA_CENTERS_PAGE = "https://epoch.ai/data/ai-data-centers";

const log = (line: string) => console.error(line);

/* ------------------------------------------------------------------ args */

type Mode = "cache" | "refresh" | "replay";
const argv = process.argv.slice(2);
const mode: Mode = argv.includes("--refresh") ? "refresh" : argv.includes("--replay") ? "replay" : "cache";
const dateArg = argv.includes("--date") ? argv[argv.indexOf("--date") + 1] : undefined;
if (dateArg !== undefined && !/^\d{4}-\d{2}-\d{2}$/.test(dateArg)) throw new Error(`--date needs YYYY-MM-DD, got ${dateArg}`);

function snapshotDate(): string {
  if (dateArg) return dateArg;
  const today = new Date().toISOString().slice(0, 10);
  if (mode === "refresh") return today;
  const existing = existsSync(CACHE_ROOT)
    ? readdirSync(CACHE_ROOT)
        .filter((d) => /^\d{4}-\d{2}-\d{2}$/.test(d) && existsSync(join(CACHE_ROOT, d, "manifest.json")))
        .sort()
    : [];
  return existing.at(-1) ?? today;
}

/* --------------------------------------------------------- cached fetcher */

interface ManifestEntry {
  url: string;
  file: string;
  fetchedAt: string;
  status: number;
  sha256: string;
  bytes: number;
  contentType?: string;
}

interface Download {
  url: string;
  bytes: Uint8Array;
  text: () => string;
  fetchedAt: string;
  sha256: string;
  fromCache: boolean;
}

const delay = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));
const sha256Hex = (bytes: Uint8Array) => createHash("sha256").update(bytes).digest("hex");

function writeAtomic(path: string, data: string | Uint8Array): void {
  const tmp = `${path}.${process.pid}.tmp`;
  writeFileSync(tmp, data);
  renameSync(tmp, path);
}

class EpochFetcher {
  private lastStartMs = 0;
  private readonly manifestPath: string;
  private manifest: Record<string, ManifestEntry>;
  readonly stats = { network: 0, cache: 0 };

  constructor(readonly dir: string) {
    this.manifestPath = join(dir, "manifest.json");
    this.manifest = existsSync(this.manifestPath) ? (JSON.parse(readFileSync(this.manifestPath, "utf8")).entries ?? {}) : {};
  }

  private fileName(url: string): string {
    const u = new URL(url);
    return `${u.hostname}${u.pathname}`.replace(/[^A-Za-z0-9._-]+/g, "_").replace(/_+$/, "");
  }

  private wrap(url: string, bytes: Uint8Array, fetchedAt: string, fromCache: boolean): Download {
    return { url, bytes, text: () => new TextDecoder().decode(bytes), fetchedAt, sha256: sha256Hex(bytes), fromCache };
  }

  async get(url: string): Promise<Download> {
    const entry = this.manifest[url];
    if (mode !== "refresh" && entry) {
      const path = join(this.dir, entry.file);
      if (existsSync(path)) {
        const bytes = new Uint8Array(readFileSync(path));
        if (sha256Hex(bytes) === entry.sha256) {
          this.stats.cache++;
          return this.wrap(url, bytes, entry.fetchedAt, true);
        }
        log(`cache: SHA-256 mismatch for ${url}; fetching again`);
      }
    }
    if (mode === "replay") throw new Error(`not in the Epoch cache (replay mode): ${url}`);
    let problem = "";
    for (let attempt = 1; attempt <= MAX_ATTEMPTS; attempt++) {
      const wait = this.lastStartMs + MIN_INTERVAL_MS - Date.now();
      if (wait > 0) await delay(wait);
      this.lastStartMs = Date.now();
      this.stats.network++;
      const fetchedAt = new Date().toISOString();
      try {
        const res = await fetch(url, { headers: { "User-Agent": USER_AGENT }, signal: AbortSignal.timeout(TIMEOUT_MS), redirect: "follow" });
        const bytes = new Uint8Array(await res.arrayBuffer());
        log(`fetch ${res.status} ${url} (${bytes.byteLength} bytes, attempt ${attempt})`);
        if (res.status === 200) {
          mkdirSync(this.dir, { recursive: true });
          const file = this.fileName(url);
          writeAtomic(join(this.dir, file), bytes);
          this.manifest[url] = {
            url,
            file,
            fetchedAt,
            status: res.status,
            sha256: sha256Hex(bytes),
            bytes: bytes.byteLength,
            ...(res.headers.get("content-type") ? { contentType: res.headers.get("content-type")! } : {}),
          };
          writeAtomic(this.manifestPath, `${JSON.stringify({ version: 1, entries: this.manifest }, null, 2)}\n`);
          return this.wrap(url, bytes, fetchedAt, false);
        }
        if (res.status !== 429 && res.status < 500) throw new Error(`HTTP ${res.status} for ${url}`);
        problem = `HTTP ${res.status}`;
      } catch (error) {
        if (error instanceof Error && error.message.startsWith("HTTP ")) throw error;
        problem = error instanceof Error ? `${error.name}: ${error.message}` : String(error);
        log(`fetch failed ${url} (attempt ${attempt}): ${problem}`);
      }
      if (attempt < MAX_ATTEMPTS) await delay(1000 * 2 ** (attempt - 1));
    }
    throw new Error(`request failed after ${MAX_ATTEMPTS} attempts (${problem}): ${url}`);
  }
}

/* ------------------------------------------------------------ page parts */

const decodeEntities = (s: string) =>
  s
    .replace(/&#x([0-9a-f]+);/gi, (_, h: string) => String.fromCodePoint(parseInt(h, 16)))
    .replace(/&#(\d+);/g, (_, d: string) => String.fromCodePoint(Number(d)))
    .replace(/&quot;/g, '"')
    .replace(/&apos;/g, "'")
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">")
    .replace(/&nbsp;/g, "\u00a0")
    .replace(/&amp;/g, "&");

const textOf = (html: string) =>
  decodeEntities(html.replace(/<!--[\s\S]*?-->/g, "").replace(/<[^>]+>/g, ""))
    .replace(/\s+/g, " ")
    .trim();

interface PageFacts {
  updatedText: string;
  updated: string;
  citation: string | null;
  licenseText: string;
}

/** The "Updated ..." badge, the citation box and the license sentence of an Epoch data page. */
function pageFacts(html: string, url: string): PageFacts {
  const badges = [...html.matchAll(/<span class="badge-text">([\s\S]*?)<\/span>/g)].map((m) => textOf(m[1]));
  const updatedText = badges.find((b) => /^Updated /.test(b));
  if (!updatedText) throw new Error(`${url}: no "Updated ..." badge found`);
  const citationHtml = /<div class="citation-text[^"]*">([\s\S]*?)<\/div>/.exec(html)?.[1];
  const licenseHtml = [...html.matchAll(/<p>([\s\S]*?)<\/p>/g)].map((m) => m[1]).find((p) => /free to use/i.test(p));
  if (!licenseHtml) throw new Error(`${url}: no license sentence found`);
  if (!licenseHtml.includes(EPOCH_LICENSE_URL)) {
    throw new Error(`${url}: the license sentence does not link to ${EPOCH_LICENSE_URL}; review the license`);
  }
  return {
    updatedText,
    updated: parseEpochUpdated(updatedText),
    citation: citationHtml ? textOf(citationHtml) : null,
    licenseText: textOf(licenseHtml),
  };
}

/** The first link on the page whose path ends with the file name, as an absolute URL. */
function downloadLink(html: string, pageUrl: string, fileName: string): string {
  const escaped = fileName.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const m = new RegExp(`(?:href|contentUrl)"?[=:]"([^"]*/${escaped})"`).exec(html);
  if (!m) throw new Error(`${pageUrl}: no download link for ${fileName}`);
  return new URL(m[1], pageUrl).toString();
}

const MONTHS_SHORT = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
/** Epoch's citation template ends with "Accessed <d Mon yyyy>."; use our access date there. */
function withAccessDate(citation: string, isoDate: string): string {
  const [y, m, d] = isoDate.split("-").map(Number);
  return citation.replace(/Accessed \d{1,2} [A-Z][a-z]{2} \d{4}\./, `Accessed ${d} ${MONTHS_SHORT[m - 1]} ${y}.`);
}

/** The dataset title inside the citation quotes: 'Data on AI companies'. */
function citationTitle(citation: string): string {
  const m = /['‘’"]([^'‘’"]+)['‘’"]\.\s*Published online/.exec(citation);
  if (!m) throw new Error(`no dataset title in citation: ${citation}`);
  return m[1];
}

/* ------------------------------------------------------------------- ZIP */

/** Files of a ZIP archive (stored or deflated), read through the central directory. */
function unzip(bytes: Uint8Array): Map<string, Uint8Array> {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  let eocd = -1;
  for (let i = bytes.length - 22; i >= Math.max(0, bytes.length - 22 - 0xffff); i--) {
    if (view.getUint32(i, true) === 0x06054b50) {
      eocd = i;
      break;
    }
  }
  if (eocd < 0) throw new Error("not a ZIP archive");
  const count = view.getUint16(eocd + 10, true);
  let p = view.getUint32(eocd + 16, true);
  const files = new Map<string, Uint8Array>();
  for (let n = 0; n < count; n++) {
    if (view.getUint32(p, true) !== 0x02014b50) throw new Error("bad ZIP central directory");
    const method = view.getUint16(p + 10, true);
    const compressedSize = view.getUint32(p + 20, true);
    const size = view.getUint32(p + 24, true);
    const nameLength = view.getUint16(p + 28, true);
    const extraLength = view.getUint16(p + 30, true);
    const commentLength = view.getUint16(p + 32, true);
    const local = view.getUint32(p + 42, true);
    const name = new TextDecoder().decode(bytes.subarray(p + 46, p + 46 + nameLength));
    if (view.getUint32(local, true) !== 0x04034b50) throw new Error(`bad ZIP local header for ${name}`);
    const start = local + 30 + view.getUint16(local + 26, true) + view.getUint16(local + 28, true);
    const data = bytes.subarray(start, start + compressedSize);
    const content = method === 0 ? data : method === 8 ? new Uint8Array(inflateRawSync(data)) : null;
    if (!content) throw new Error(`unsupported ZIP compression method ${method} for ${name}`);
    if (content.byteLength !== size) throw new Error(`ZIP size mismatch for ${name}`);
    if (!name.endsWith("/")) files.set(name, content);
    p += 46 + nameLength + extraLength + commentLength;
  }
  return files;
}

/* -------------------------------------------------------------- content */

function loadOrgs(): Org[] {
  const files = readdirSync(CONTENT).filter((name) => /^orgs(?:-[a-z0-9-]+)?\.json$/.test(name));
  return files.flatMap((file) => {
    const parsed = orgsFileSchema.safeParse(JSON.parse(readFileSync(join(CONTENT, file), "utf8")));
    if (!parsed.success) throw new Error(`${file}: ${parsed.error.issues[0]?.path.join(".")}: ${parsed.error.issues[0]?.message}`);
    return parsed.data.orgs;
  });
}

/** Valid hand-written events. A file that does not parse (another editor mid-change) is skipped with a warning. */
function loadEvents(): CapitalEvent[] {
  const dir = join(CONTENT, "events");
  if (!existsSync(dir)) return [];
  const events: CapitalEvent[] = [];
  for (const file of readdirSync(dir).filter((f) => f.endsWith(".json")).sort()) {
    try {
      const parsed = eventFileSchema.safeParse(JSON.parse(readFileSync(join(dir, file), "utf8")));
      if (parsed.success) events.push(parsed.data.event);
      else log(`warning: events/${file} does not parse (${parsed.error.issues[0]?.path.join(".")}: ${parsed.error.issues[0]?.message}); skipped`);
    } catch (error) {
      log(`warning: events/${file} is not JSON (${error instanceof Error ? error.message : String(error)}); skipped`);
    }
  }
  return events;
}

/* ------------------------------------------------------------------ main */

async function main() {
  const date = snapshotDate();
  const fetcher = new EpochFetcher(join(CACHE_ROOT, date));
  log(`Epoch snapshot ${date} (${mode} mode), cache ${fetcher.dir}`);

  const orgs = loadOrgs();
  const index = buildOrgIndex(orgs);
  const events = loadEvents();
  log(`${orgs.length} orgs, ${events.length} events.`);

  /* ---- funding rounds */
  const companiesPage = await fetcher.get(COMPANIES_PAGE);
  const companiesHtml = companiesPage.text();
  const companies = pageFacts(companiesHtml, COMPANIES_PAGE);
  const fundingUrl = downloadLink(companiesHtml, COMPANIES_PAGE, "ai_companies_funding_rounds.csv");
  const fundingCsv = await fetcher.get(fundingUrl);
  if (!companies.citation) throw new Error(`${COMPANIES_PAGE}: no citation found`);
  const fundingWarnings: string[] = [];
  const fundingRows = csvRecords(fundingCsv.text(), EPOCH_FUNDING_COLUMNS).map((record) => {
    const { row, warnings } = normalizeFundingRow(record, index);
    fundingWarnings.push(...warnings);
    const matches = findMatchingEvents(row, events);
    if (matches.length > 1) fundingWarnings.push(`${row.epochId}: several events match (${matches.join(", ")}); left unreviewed`);
    return { ...row, review: reviewFor(matches.length === 1 ? matches[0] : null) };
  });
  const fundingRetrieved = fundingCsv.fetchedAt.slice(0, 10);
  const funding: EpochFundingInbox = epochFundingInboxSchema.parse({
    dataset: `Epoch AI, ${citationTitle(companies.citation)}`,
    url: COMPANIES_PAGE,
    downloadUrl: fundingUrl,
    license: EPOCH_LICENSE,
    licenseUrl: EPOCH_LICENSE_URL,
    licenseText: companies.licenseText,
    attribution: withAccessDate(companies.citation, fundingRetrieved),
    retrievedAt: fundingRetrieved,
    epochUpdated: companies.updated,
    epochUpdatedText: companies.updatedText,
    sha256: fundingCsv.sha256,
    note: EPOCH_INBOX_NOTE,
    rows: fundingRows,
  });

  /* ---- data centers */
  const centersPage = await fetcher.get(DATA_CENTERS_PAGE);
  const centersHtml = centersPage.text();
  const centers = pageFacts(centersHtml, DATA_CENTERS_PAGE);
  const zipUrl = downloadLink(centersHtml, DATA_CENTERS_PAGE, "data_centers.zip");
  const zip = await fetcher.get(zipUrl);
  const files = unzip(zip.bytes);
  const readme = files.get("README.md");
  const centersCsv = files.get("data_centers.csv");
  if (!readme || !centersCsv) throw new Error(`data_centers.zip lacks README.md or data_centers.csv (has ${[...files.keys()].join(", ")})`);
  const readmeText = new TextDecoder().decode(readme);
  if (!readmeText.includes(EPOCH_LICENSE_URL)) throw new Error("README.md does not link to the CC BY 4.0 license; review the license");
  const readmeCitation = /### Citation\s*```\s*([\s\S]*?)```/.exec(readmeText)?.[1].trim() ?? null;
  const centersCitation = centers.citation ?? readmeCitation;
  if (!centersCitation) throw new Error("no citation for the data centers dataset");
  const centerWarnings: string[] = [];
  const centerRows = csvRecords(new TextDecoder().decode(centersCsv), EPOCH_DATA_CENTER_COLUMNS).map((record) => {
    const { row, warnings } = normalizeDataCenterRow(record, index);
    centerWarnings.push(...warnings);
    return row;
  });
  const centersRetrieved = zip.fetchedAt.slice(0, 10);
  const dataCenters: EpochDataCenterInbox = epochDataCenterInboxSchema.parse({
    dataset: `Epoch AI, ${citationTitle(centersCitation)}`,
    url: DATA_CENTERS_PAGE,
    downloadUrl: zipUrl,
    license: EPOCH_LICENSE,
    licenseUrl: EPOCH_LICENSE_URL,
    licenseText: centers.licenseText,
    attribution: withAccessDate(centersCitation, centersRetrieved),
    retrievedAt: centersRetrieved,
    epochUpdated: centers.updated,
    epochUpdatedText: centers.updatedText,
    sha256: zip.sha256,
    note: EPOCH_INBOX_NOTE,
    rows: centerRows,
  });

  /* ---- write */
  mkdirSync(INBOX, { recursive: true });
  writeFileSync(join(INBOX, "epoch-funding-rounds.json"), `${JSON.stringify(funding, null, 2)}\n`);
  writeFileSync(join(INBOX, "epoch-data-centers.json"), `${JSON.stringify(dataCenters, null, 2)}\n`);

  /* ---- report */
  const count = <T>(rows: T[], test: (row: T) => boolean) => rows.filter(test).length;
  const unmapped = (names: string[]) => [...new Set(names)].sort().join(", ") || "none";
  console.log(`Snapshot ${date}. Cache: ${fetcher.stats.network} fetched, ${fetcher.stats.cache} from disk.`);
  const published = funding.rows.filter((r) => r.review.event).map((r) => `${r.epochId} -> ${r.review.event}`);
  console.log(
    [
      `Funding rounds (${funding.epochUpdatedText}, sha256 ${funding.sha256.slice(0, 12)}): ${funding.rows.length} rows,`,
      `${count(funding.rows, (r) => r.org !== null)} mapped to an org,`,
      `${published.length} published (${published.join("; ") || "none"}),`,
      `${count(funding.rows, (r) => r.proposed.length > 0)} with proposed amounts.`,
    ].join(" ")
  );
  console.log(`  Unmapped companies: ${unmapped(funding.rows.filter((r) => !r.org).map((r) => r.company))}`);
  console.log(
    `Data centers (${dataCenters.epochUpdatedText}, sha256 ${dataCenters.sha256.slice(0, 12)}): ` +
      `${dataCenters.rows.length} rows, ${count(dataCenters.rows, (r) => r.org !== null)} with a mapped owner.`
  );
  console.log(`  Unmapped owners: ${unmapped(dataCenters.rows.filter((r) => r.owner && !r.org).map((r) => r.owner!))}`);
  console.log(`  ZIP files: ${[...files.keys()].join(", ")}`);
  console.log(`License: ${funding.licenseText}`);
  console.log(`Attribution (funding): ${funding.attribution}`);
  console.log(`Attribution (data centers): ${dataCenters.attribution}`);
  for (const w of [...fundingWarnings, ...centerWarnings]) console.log(`warning: ${w}`);
}

await main();
