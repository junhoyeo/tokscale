/**
 * Verify and fill identifiers for the Capital v2 orgs.
 *
 *   bun scripts/capital/build-org-ids.ts             # cached responses first; fetch only what is missing
 *   bun scripts/capital/build-org-ids.ts --refresh   # fetch every response again
 *   bun scripts/capital/build-org-ids.ts --replay    # offline; a missing response is an error
 *   bun scripts/capital/build-org-ids.ts --out DIR   # default /tmp/tokscale-stealth/capital2/helpers
 *
 * Input (read-only): every org in src/content/capital/orgs*.json.
 * Output: DIR/org-ids.json (only verified fields) and DIR/org-ids.md (counts and every difference).
 *
 * What is checked, per org:
 * - Wikidata QID. Candidates come from a name search (name, legal names, aliases), from P5531 = our
 *   CIK, and, for orgs still without a match, from P856 = our website. Items that are not
 *   organizations (products, places, people: judged by their P31 classes) are dropped. A QID is
 *   accepted only on a strong match: P856 official website host equals ours (plus a name match),
 *   P5531 (SEC CIK) equals one of our CIKs, or P414 exchange + P249 ticker equal one of our tickers.
 *   A plain label match is accepted only when the description fits the org's category, the country
 *   does not conflict, and no other candidate qualifies; such picks are listed as "please confirm".
 *   Then: P571 inception (with its precision), P159 headquarters (city only if the item is a city),
 *   country as ISO 3166-1 alpha-2 (P297 of the item's P17, else of the HQ's P17), and P856 website.
 * - Hugging Face org: /api/organizations/<name>/overview returns exactly that name.
 * - GitHub org: /orgs/<login> exists and its name or blog domain matches the org.
 * - SEC CIKs already listed: the submissions record resolves and its name matches the org.
 *
 * Network rules: descriptive User-Agent, at most 2 requests per second per host (Wikidata, Hugging
 * Face, GitHub; SEC through the shared SecClient gate), 30 s timeouts, limited retries on 429/5xx.
 * Every response is cached under .stealth-cache/capital/<host>/ with a manifest (url, fetchedAt,
 * status, sha256), so reruns replay from disk.
 */
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, readdirSync, renameSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { orgsFileSchema, type Org } from "@/lib/capital/ledger/schema";
import { SecClient, padCik, submissionsUrl } from "./lib/sec-client";

const ROOT = join(import.meta.dir, "..", "..");
const CONTENT = join(ROOT, "src", "content", "capital");
const CACHE_ROOT = join(ROOT, ".stealth-cache", "capital");
const USER_AGENT = "TokscaleResearch/1.0 (i@junho.io)";
const TIMEOUT_MS = 30_000;
const MAX_ATTEMPTS = 4;
const MAX_BACKOFF_MS = 30_000;

/* ------------------------------------------------------------------ args */

type Mode = "cache" | "refresh" | "replay";
const argv = process.argv.slice(2);
const mode: Mode = argv.includes("--refresh") ? "refresh" : argv.includes("--replay") ? "replay" : "cache";
const outIndex = argv.indexOf("--out");
const OUT_DIR = outIndex >= 0 ? argv[outIndex + 1] : "/tmp/tokscale-stealth/capital2/helpers";
const log = (line: string) => console.error(line);

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

interface Fetched {
  url: string;
  status: number;
  body: string;
  fetchedAt: string;
  fromCache: boolean;
}

class HttpError extends Error {
  constructor(
    readonly url: string,
    readonly status: number,
    message: string
  ) {
    super(message);
    this.name = "HttpError";
  }
}

const delay = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));
const sha256Hex = (bytes: Uint8Array) => createHash("sha256").update(bytes).digest("hex");

function cacheFileName(url: string): string {
  const u = new URL(url);
  const base = `${u.hostname}${u.pathname}`.replace(/[^A-Za-z0-9._-]+/g, "_").replace(/_+$/, "");
  const query = u.search ? `__${createHash("sha256").update(u.search).digest("hex").slice(0, 16)}` : "";
  return `${base.slice(-150)}${query}`;
}

function writeAtomic(path: string, data: string | Uint8Array): void {
  const tmp = `${path}.${process.pid}.tmp`;
  writeFileSync(tmp, data);
  renameSync(tmp, path);
}

function retryAfterMs(header: string | null): number | null {
  if (!header) return null;
  const seconds = Number(header);
  if (Number.isFinite(seconds) && seconds >= 0) return Math.min(seconds * 1000, MAX_BACKOFF_MS);
  const at = Date.parse(header);
  return Number.isFinite(at) ? Math.min(Math.max(at - Date.now(), 0), MAX_BACKOFF_MS) : null;
}

interface FetcherOptions {
  name: string;
  dir: string;
  minIntervalMs: number;
  /** Statuses worth saving (stable answers). Others are never cached. */
  cacheable: ReadonlySet<number>;
  /** A 200 body that must be retried instead of cached (Wikidata "maxlag"). */
  retryBody?: (body: string) => boolean;
  /** Statuses that mean "stop, you are rate limited" (GitHub 403 with no requests left). */
  fatal?: (status: number, headers: Headers) => string | null;
}

/** One host's polite fetcher: paced, bounded, retried, and cached on disk with a manifest. */
class CachedFetcher {
  readonly stats = { network: 0, cache: 0, retries: 0 };
  private gate: Promise<void> = Promise.resolve();
  private lastStartMs = 0;
  private manifest: Record<string, ManifestEntry>;

  constructor(private readonly o: FetcherOptions) {
    const path = join(o.dir, "manifest.json");
    this.manifest = existsSync(path) ? (JSON.parse(readFileSync(path, "utf8")).entries ?? {}) : {};
  }

  private nextSlot(): Promise<void> {
    const turn = this.gate.then(async () => {
      const wait = this.lastStartMs + this.o.minIntervalMs - Date.now();
      if (wait > 0) await delay(wait);
      this.lastStartMs = Date.now();
    });
    this.gate = turn.catch(() => undefined);
    return turn;
  }

  private fromCache(url: string): Fetched | null {
    const entry = this.manifest[url];
    if (!entry) return null;
    const path = join(this.o.dir, entry.file);
    if (!existsSync(path)) return null;
    const bytes = new Uint8Array(readFileSync(path));
    if (sha256Hex(bytes) !== entry.sha256) {
      log(`[${this.o.name}] cache: SHA-256 mismatch, ignoring saved copy of ${url}`);
      return null;
    }
    return { url, status: entry.status, body: new TextDecoder().decode(bytes), fetchedAt: entry.fetchedAt, fromCache: true };
  }

  private record(url: string, status: number, bytes: Uint8Array, fetchedAt: string, contentType: string | null): void {
    mkdirSync(this.o.dir, { recursive: true });
    const file = cacheFileName(url);
    writeAtomic(join(this.o.dir, file), bytes);
    this.manifest[url] = {
      url,
      file,
      fetchedAt,
      status,
      sha256: sha256Hex(bytes),
      bytes: bytes.byteLength,
      ...(contentType ? { contentType } : {}),
    };
    const sorted = Object.fromEntries(Object.entries(this.manifest).sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0)));
    writeAtomic(join(this.o.dir, "manifest.json"), `${JSON.stringify({ version: 1, entries: sorted }, null, 2)}\n`);
  }

  async get(url: string): Promise<Fetched> {
    if (mode !== "refresh") {
      const cached = this.fromCache(url);
      if (cached) {
        this.stats.cache++;
        return cached;
      }
      if (mode === "replay") throw new HttpError(url, 0, `not in the ${this.o.name} cache (replay mode): ${url}`);
    }
    let problem = "";
    for (let attempt = 1; attempt <= MAX_ATTEMPTS; attempt++) {
      await this.nextSlot();
      this.stats.network++;
      let waitMs: number | null = null;
      const fetchedAt = new Date().toISOString();
      try {
        const res = await fetch(url, {
          headers: { "User-Agent": USER_AGENT, Accept: "application/json" },
          signal: AbortSignal.timeout(TIMEOUT_MS),
          redirect: "follow",
        });
        const bytes = new Uint8Array(await res.arrayBuffer());
        const body = new TextDecoder().decode(bytes);
        log(`[${this.o.name}] fetch ${res.status} ${url} (${bytes.byteLength} bytes, attempt ${attempt})`);
        const fatal = this.o.fatal?.(res.status, res.headers);
        if (fatal) throw new HttpError(url, res.status, fatal);
        const retryStatus = res.status === 429 || res.status >= 500;
        const retryBody = res.status === 200 && this.o.retryBody?.(body);
        if (!retryStatus && !retryBody) {
          if (this.o.cacheable.has(res.status)) this.record(url, res.status, bytes, fetchedAt, res.headers.get("content-type"));
          return { url, status: res.status, body, fetchedAt, fromCache: false };
        }
        problem = retryBody ? "server asked to retry (maxlag)" : `HTTP ${res.status}`;
        waitMs = retryAfterMs(res.headers.get("retry-after"));
      } catch (error) {
        if (error instanceof HttpError) throw error;
        problem = error instanceof Error ? `${error.name}: ${error.message}` : String(error);
        log(`[${this.o.name}] fetch failed ${url} (attempt ${attempt}): ${problem}`);
      }
      if (attempt < MAX_ATTEMPTS) {
        this.stats.retries++;
        await delay(waitMs ?? Math.min(1000 * 2 ** (attempt - 1), MAX_BACKOFF_MS));
      }
    }
    throw new HttpError(url, 0, `${this.o.name}: request failed after ${MAX_ATTEMPTS} attempts (${problem}): ${url}`);
  }

  /** URLs with a saved 200 response. */
  cachedUrls(): string[] {
    return Object.values(this.manifest)
      .filter((e) => e.status === 200)
      .map((e) => e.url);
  }

  async getJson<T>(url: string): Promise<{ status: number; data: T | null; fetchedAt: string }> {
    const r = await this.get(url);
    if (r.status !== 200) return { status: r.status, data: null, fetchedAt: r.fetchedAt };
    return { status: r.status, data: JSON.parse(r.body) as T, fetchedAt: r.fetchedAt };
  }
}

const wikidata = new CachedFetcher({
  name: "wikidata",
  dir: join(CACHE_ROOT, "wikidata"),
  minIntervalMs: 500,
  cacheable: new Set([200]),
  retryBody: (body) => body.includes('"code":"maxlag"'),
});
const huggingface = new CachedFetcher({
  name: "huggingface",
  dir: join(CACHE_ROOT, "huggingface"),
  minIntervalMs: 500,
  cacheable: new Set([200, 404]),
});
const github = new CachedFetcher({
  name: "github",
  dir: join(CACHE_ROOT, "github"),
  minIntervalMs: 1000,
  cacheable: new Set([200, 404]),
  fatal: (status, headers) =>
    (status === 403 || status === 429) && headers.get("x-ratelimit-remaining") === "0"
      ? `GitHub rate limit reached (resets at ${new Date(Number(headers.get("x-ratelimit-reset")) * 1000).toISOString()})`
      : null,
});
const sec = new SecClient({ mode, log: (line) => log(`[sec] ${line}`) });

/* ------------------------------------------------------------------- orgs */

function loadOrgs(): { orgs: Org[]; files: string[] } {
  const files = readdirSync(CONTENT)
    .filter((name) => /^orgs(?:-[a-z0-9-]+)?\.json$/.test(name))
    .sort();
  const orgs: Org[] = [];
  for (const file of files) {
    const parsed = orgsFileSchema.safeParse(JSON.parse(readFileSync(join(CONTENT, file), "utf8")));
    if (!parsed.success) throw new Error(`${file}: ${parsed.error.issues[0]?.path.join(".")}: ${parsed.error.issues[0]?.message}`);
    orgs.push(...parsed.data.orgs);
  }
  return { orgs, files };
}

/* ---------------------------------------------------------- name matching */

/** Legal-form words only. "Group" and "Holdings" stay: "Pershing Square Holdings" is not "Pershing Square". */
const LEGAL_SUFFIXES = new Set([
  "inc", "incorporated", "corp", "corporation", "co", "company", "ltd", "limited", "llc", "lp", "llp", "plc", "pbc",
  "nv", "bv", "sa", "ag", "se", "pte", "gmbh", "kk",
]);

/** Lowercase words without punctuation: "Amazon.com, Inc." -> "amazon com inc". */
const words = (text: string) =>
  text
    .normalize("NFKD")
    .replace(/[\u0300-\u036f]/g, "")
    .replace(/\bL\.\s?L\.\s?C\.?/gi, "LLC")
    .replace(/\bL\.\s?P\.?(?=\s|$|,)/gi, "LP")
    .replace(/\bN\.\s?V\.?(?=\s|$|,)/gi, "NV")
    .toLowerCase()
    .replace(/&/g, " and ")
    .replace(/[^a-z0-9]+/g, " ")
    .trim()
    .split(/\s+/)
    .filter(Boolean);

/** Words without a leading "the" and trailing legal-form words: "Anysphere, Inc." -> "anysphere". */
const coreName = (text: string) => {
  const w = words(text);
  if (w.length > 1 && w[0] === "the") w.shift();
  while (w.length > 1 && LEGAL_SUFFIXES.has(w[w.length - 1])) w.pop();
  return w.join(" ");
};

const orgNames = (org: Org) => [org.name, ...(org.shortName ? [org.shortName] : []), ...org.legalNames, ...org.aliases];

/** True when one of the org's names equals the text (ignoring case, punctuation and corporate suffixes). */
function sameName(org: Org, text: string): boolean {
  const t = coreName(text);
  return t.length > 0 && orgNames(org).some((n) => coreName(n) === t);
}

/** Words that name a corporate structure, not an entity ("Group", "Holdings"). */
const STRUCTURE_WORDS = new Set(["group", "holding", "holdings", "companies", "the"]);

/**
 * SEC conformed names may reorder a person's name and add "Group" ("PRICE T ROWE GROUP INC" for
 * T. Rowe Price). True when the words match in any order, ignoring legal-form and structure words.
 * Any other extra word (an SPV's "Series I", "Fund", a broker's name) still fails.
 */
function sameWordsAnyOrder(org: Org, text: string): boolean {
  const bag = (t: string) => new Set(words(t).filter((w) => !LEGAL_SUFFIXES.has(w) && !STRUCTURE_WORDS.has(w)));
  const theirs = bag(text);
  return (
    theirs.size > 0 &&
    orgNames(org).some((n) => {
      const ours = bag(n);
      return ours.size === theirs.size && [...ours].every((w) => theirs.has(w));
    })
  );
}

/** True when every word of the org's main name appears in the text ("Crusoe" in "Crusoe Energy Systems"). */
function containsName(org: Org, text: string): boolean {
  const have = new Set(words(text));
  const need = words(org.name);
  return need.length > 0 && need.every((w) => have.has(w));
}

const hostOf = (url: string | undefined | null): string | null => {
  if (!url) return null;
  try {
    return new URL(/^https?:\/\//i.test(url) ? url : `https://${url}`).hostname.toLowerCase().replace(/^www\./, "");
  } catch {
    return null;
  }
};

/** Last two labels of a host, or three for two-level public suffixes like co.uk and com.cn. */
const registrableDomain = (host: string | null): string | null => {
  if (!host) return null;
  const parts = host.split(".");
  const twoLevel = /^(co|com|net|org|ac|gov|ne|or)\.[a-z]{2}$/.test(parts.slice(-2).join("."));
  return parts.slice(twoLevel ? -3 : -2).join(".");
};

/* --------------------------------------------------------------- wikidata */

const WD_API = "https://www.wikidata.org/w/api.php";
const wdUrl = (params: Record<string, string>) =>
  `${WD_API}?${new URLSearchParams({ format: "json", formatversion: "2", maxlag: "5", ...params }).toString()}`;

interface WdSnak {
  snaktype: string;
  property: string;
  datavalue?: { type: string; value: unknown };
}
interface WdClaim {
  mainsnak: WdSnak;
  rank: "preferred" | "normal" | "deprecated";
  qualifiers?: Record<string, WdSnak[]>;
}
interface WdEntity {
  id: string;
  missing?: boolean | string;
  labels?: Record<string, { value: string }>;
  descriptions?: Record<string, { value: string }>;
  aliases?: Record<string, Array<{ value: string }>>;
  claims?: Record<string, WdClaim[]>;
}

const label = (e: WdEntity | undefined) => e?.labels?.en?.value ?? "";
const description = (e: WdEntity | undefined) => e?.descriptions?.en?.value ?? "";
const aliasesOf = (e: WdEntity | undefined) => (e?.aliases?.en ?? []).map((a) => a.value);

/** Current claims: not deprecated, no end time (P582); the preferred ones when any is preferred. */
function currentClaims(e: WdEntity | undefined, property: string): WdClaim[] {
  const all = (e?.claims?.[property] ?? []).filter(
    (c) => c.rank !== "deprecated" && c.mainsnak.snaktype === "value" && !(c.qualifiers?.P582?.length)
  );
  const preferred = all.filter((c) => c.rank === "preferred");
  return preferred.length > 0 ? preferred : all;
}

/** Every non-deprecated value, including ended ones (for matching, not for output). */
const anyClaims = (e: WdEntity | undefined, property: string) =>
  (e?.claims?.[property] ?? []).filter((c) => c.rank !== "deprecated" && c.mainsnak.snaktype === "value");

const stringValue = (c: WdClaim) => (typeof c.mainsnak.datavalue?.value === "string" ? (c.mainsnak.datavalue.value as string) : null);
const itemValue = (c: WdClaim) => {
  const v = c.mainsnak.datavalue?.value as { id?: string } | undefined;
  return v && typeof v.id === "string" ? v.id : null;
};
const unique = <T>(values: T[]) => [...new Set(values)];

interface SearchHit {
  id: string;
  label?: string;
  description?: string;
}

const entityCache = new Map<string, WdEntity>();
const entityFetchedAt = new Map<string, string>();

async function wdSearch(term: string): Promise<SearchHit[]> {
  const { data } = await wikidata.getJson<{ search?: SearchHit[]; error?: { info: string } }>(
    wdUrl({ action: "wbsearchentities", search: term, language: "en", uselang: "en", type: "item", limit: "7" })
  );
  if (!data || data.error) throw new Error(`Wikidata search failed for "${term}": ${data?.error?.info ?? "no data"}`);
  return data.search ?? [];
}

/** Items with an exact statement value, through CirrusSearch haswbstatement ("P5531=0001652044"). */
async function wdByStatement(statement: string): Promise<string[]> {
  const { data } = await wikidata.getJson<{ query?: { search?: Array<{ title: string }> } }>(
    wdUrl({ action: "query", list: "search", srsearch: `haswbstatement:${statement}`, srlimit: "5", srnamespace: "0" })
  );
  return unique((data?.query?.search ?? []).map((s) => s.title).filter((t) => /^Q\d+$/.test(t)));
}

/** Items whose P5531 (SEC Central Index Key) equals the CIK, zero-padded or not. */
async function wdByCik(cik: string): Promise<string[]> {
  for (const value of unique([padCik(cik), String(Number(cik))])) {
    const ids = await wdByStatement(`P5531=${value}`);
    if (ids.length > 0) return ids;
  }
  return [];
}

/** Items whose P856 official website is exactly one of the usual spellings of our site. */
async function wdByWebsite(website: string): Promise<string[]> {
  const host = hostOf(website);
  if (!host) return [];
  for (const variant of [`https://${host}/`, `https://${host}`, `https://www.${host}/`, `https://www.${host}`]) {
    const ids = await wdByStatement(`P856=${variant}`);
    if (ids.length > 0) return ids;
  }
  return [];
}

/** Saved wbgetentities responses by "<props><extra>:<id>", so a rerun reads every item it already has from disk. */
let savedBatches: Map<string, string> | null = null;
function savedBatchFor(key: string, id: string): string | undefined {
  if (!savedBatches) {
    savedBatches = new Map();
    if (mode !== "refresh") {
      for (const url of wikidata.cachedUrls()) {
        const params = new URL(url).searchParams;
        if (params.get("action") !== "wbgetentities") continue;
        const extra: Record<string, string> = params.has("sitefilter") ? { sitefilter: params.get("sitefilter")! } : {};
        const batchKey = `${params.get("props")}${JSON.stringify(extra)}`;
        for (const savedId of (params.get("ids") ?? "").split("|")) savedBatches.set(`${batchKey}:${savedId}`, url);
      }
    }
  }
  return savedBatches.get(`${key}:${id}`);
}

async function storeBatch(url: string, key: string, props: string): Promise<void> {
  const { data, fetchedAt } = await wikidata.getJson<{ entities?: Record<string, WdEntity>; error?: { info: string } }>(url);
  if (!data?.entities) throw new Error(`Wikidata wbgetentities failed: ${data?.error?.info ?? "no data"}`);
  for (const [id, entity] of Object.entries(data.entities)) {
    entityCache.set(`${key}:${id}`, entity);
    if (props === FULL && !entityFetchedAt.has(entity.id ?? id)) entityFetchedAt.set(entity.id ?? id, fetchedAt);
  }
}

/** Fetch entities in sorted batches. Items already saved in any earlier batch are read from disk. */
async function wdEntities(ids: string[], props: string, extra: Record<string, string> = {}, batchSize = 20): Promise<void> {
  const key = `${props}${JSON.stringify(extra)}`;
  const wanted = unique(ids).filter((id) => !entityCache.has(`${key}:${id}`));
  for (const url of unique(wanted.map((id) => savedBatchFor(key, id)).filter((u): u is string => !!u))) await storeBatch(url, key, props);
  const missing = wanted
    .filter((id) => !entityCache.has(`${key}:${id}`))
    .sort((a, b) => Number(a.slice(1)) - Number(b.slice(1)));
  for (let i = 0; i < missing.length; i += batchSize) {
    const batch = missing.slice(i, i + batchSize);
    await storeBatch(wdUrl({ action: "wbgetentities", ids: batch.join("|"), props, languages: "en", ...extra }), key, props);
  }
}

const FULL = "labels|descriptions|aliases|claims";
const SITELINKS = { props: "sitelinks", extra: { sitefilter: "enwiki" } };
const getEntity = (id: string) => entityCache.get(`${FULL}{}:${id}`);
const getLabelOnly = (id: string) => entityCache.get(`labels{}:${id}`) ?? getEntity(id);
const hasEnwiki = (id: string) => {
  const e = entityCache.get(`${SITELINKS.props}${JSON.stringify(SITELINKS.extra)}:${id}`) as (WdEntity & { sitelinks?: Record<string, unknown> }) | undefined;
  return !!e?.sitelinks?.enwiki;
};

const countryIdOf = (e: WdEntity | undefined) => unique(currentClaims(e, "P17").map(itemValue).filter((v): v is string => !!v));

/** ISO 3166-1 alpha-2 of a country item (P297), or of the country it belongs to when it has none (Netherlands -> Kingdom). */
function isoOf(countryId: string): string | null {
  const codes = (id: string) =>
    unique(currentClaims(getEntity(id), "P297").map(stringValue).filter((v): v is string => !!v && /^[A-Z]{2}$/.test(v)));
  const own = codes(countryId);
  if (own.length === 1) return own[0];
  if (own.length > 1) return null;
  const parents = countryIdOf(getEntity(countryId)).filter((p) => p !== countryId);
  if (parents.length !== 1) return null;
  const inherited = codes(parents[0]);
  return inherited.length === 1 ? inherited[0] : null;
}

/** Fetch the country items (and their parent countries) that the given items point to with P17. */
async function wdCountries(itemIds: string[]): Promise<void> {
  const countries = unique(itemIds.flatMap((id) => countryIdOf(getEntity(id))));
  await wdEntities(countries, FULL);
  const parents = unique(countries.flatMap((id) => countryIdOf(getEntity(id)))).filter((id) => !countries.includes(id));
  await wdEntities(parents, FULL);
}

/** Exchange label patterns for our ticker exchange codes. */
const EXCHANGE_LABEL: Record<string, RegExp> = {
  NASDAQ: /^nasdaq\b/i,
  NYSE: /^new york stock exchange$/i,
  HKEX: /hong kong (stock )?exchange|stock exchange of hong kong|^hkex$/i,
  TWSE: /^taiwan stock exchange$/i,
  LSE: /^london stock exchange$/i,
  TSE: /^tokyo stock exchange$/i,
};

/** Words a fitting description must contain, by org category or investor type. */
function descriptionFits(org: Org, text: string): boolean {
  const patterns: RegExp[] = [];
  for (const category of org.categories) {
    if (category === "ai-lab" || category === "ai-app") patterns.push(/artificial intelligence|\bAI\b|machine learning|language model/i);
    if (category === "hyperscaler" || category === "cloud" || category === "neocloud")
      patterns.push(/cloud|comput|data cent|technology|internet|software/i);
    if (category === "semiconductors") patterns.push(/semiconductor|chip|processor|integrated circuit|foundry|electronic/i);
    if (category === "data-centers") patterns.push(/data cent|bitcoin|mining|comput/i);
    if (category === "power") patterns.push(/energy|electric|power|utility|nuclear|reactor/i);
  }
  if (org.kinds.includes("investor"))
    patterns.push(
      /venture|private equity|investment|investor|asset manag|hedge fund|growth equity|sovereign wealth|\bfund\b|capital firm|financial services/i
    );
  return patterns.some((p) => p.test(text));
}

/** P31 class labels that make an item an organization (a company, firm, fund, lab...), not a product or place. */
const ORG_CLASS = new RegExp(
  "\\b(" +
    [
      "company", "business", "enterprise", "organi[sz]ation", "corporation", "firm", "fund", "start-?up", "subsidiary",
      "conglomerate", "partnership", "trust", "bank", "laborator(?:y|ies)", "institute", "holding", "division",
      "manufacturer", "cooperative", "venture capital", "private equity", "asset manag\\w*", "unicorn", "non-?profit",
      "foundation", "public benefit",
    ].join("|") +
    ")\\b",
  "i"
);

interface Candidate {
  id: string;
  label: string;
  description: string;
  website: string | null;
  cik: string | null;
  ticker: string | null;
  nameMatch: "exact" | "contains" | null;
  /** The English label equals our main name (not only a legal name or alias). */
  labelIsName: boolean;
  descFits: boolean;
  /** At least one P31 class is an organization class. */
  isOrg: boolean;
  classes: string[];
  /** ISO codes of the item's current P17 countries. */
  countries: string[];
}

function strongCount(c: Candidate): number {
  return (c.website ? 1 : 0) + (c.cik ? 1 : 0) + (c.ticker ? 1 : 0);
}

function evaluate(org: Org, entity: WdEntity): Candidate {
  const names = [label(entity), ...aliasesOf(entity)];
  const ourHost = hostOf(org.website);
  const websites = anyClaims(entity, "P856").map(stringValue).filter((v): v is string => !!v);
  const website = ourHost ? (websites.find((w) => hostOf(w) === ourHost) ?? null) : null;
  const ciks = anyClaims(entity, "P5531").map(stringValue).filter((v): v is string => !!v);
  const cik = ciks.find((v) => /^\d+$/.test(v) && org.ids.secCik.some((ours) => Number(ours) === Number(v))) ?? null;

  let ticker: string | null = null;
  const symbols = (snaks: WdSnak[] | undefined) =>
    (snaks ?? []).map((s) => (typeof s.datavalue?.value === "string" ? (s.datavalue.value as string) : null)).filter((v): v is string => !!v);
  const directTickers = anyClaims(entity, "P249").map(stringValue).filter((v): v is string => !!v);
  for (const claim of anyClaims(entity, "P414")) {
    const exchange = itemValue(claim);
    const exchangeLabel = exchange ? label(getLabelOnly(exchange)) : "";
    const tickers = [...symbols(claim.qualifiers?.P249), ...directTickers];
    for (const ours of org.ids.tickers) {
      if (!EXCHANGE_LABEL[ours.exchange]?.test(exchangeLabel)) continue;
      if (tickers.some((t) => t.toUpperCase() === ours.symbol.toUpperCase())) ticker = `${ours.exchange}:${ours.symbol} (${exchangeLabel})`;
    }
  }

  const nameMatch = names.some((n) => sameName(org, n)) ? "exact" : names.some((n) => containsName(org, n)) ? "contains" : null;
  const classes = anyClaims(entity, "P31")
    .map(itemValue)
    .filter((v): v is string => !!v)
    .map((id) => label(getLabelOnly(id)) || id);
  return {
    id: entity.id,
    label: label(entity),
    description: description(entity),
    website,
    cik,
    ticker,
    nameMatch,
    labelIsName: !!label(entity) && coreName(label(entity)) === coreName(org.name),
    descFits: descriptionFits(org, description(entity)),
    // An item without any P31 class is judged by its description ("American investment firm").
    isOrg: classes.length > 0 ? classes.some((c) => ORG_CLASS.test(c)) : ORG_CLASS.test(description(entity)),
    classes,
    countries: unique(countryIdOf(entity).map(isoOf).filter((v): v is string => !!v)),
  };
}

interface WikidataPick {
  candidate: Candidate | null;
  evidence: string;
  rejected: string[];
  /** True when the pick rests on a label match and a fitting description only. */
  weak: boolean;
}

const describe = (c: Candidate) => `${c.id} "${c.label}" (${c.description || "no description"})`;

function pick(org: Org, all: Candidate[], hasEnwiki: (id: string) => boolean): WikidataPick {
  // Products, places and people are dropped. Only those that shared a strong signal are worth a note.
  const notOrgs = all.filter((c) => !c.isOrg && (c.cik || c.ticker || c.website));
  const rejected = notOrgs.slice(0, 3).map((c) => `not an organization item: ${describe(c)} [${c.classes.slice(0, 3).join(", ")}]`);
  const candidates = all.filter((c) => c.isOrg);
  // A website-only match also needs a name match, so a subsidiary or product item on the same domain loses.
  const strong = candidates.filter((c) => c.cik || c.ticker || (c.website && c.nameMatch));
  if (strong.length > 0) {
    const score = (c: Candidate) => [strongCount(c), Number(c.labelIsName), Number(c.nameMatch === "exact")];
    const cmp = (a: Candidate, b: Candidate) => {
      const [x, y] = [score(a), score(b)];
      for (let i = 0; i < x.length; i++) if (x[i] !== y[i]) return y[i] - x[i];
      return 0;
    };
    const ranked = [...strong].sort(cmp);
    let tied = ranked.filter((c) => cmp(c, ranked[0]) === 0);
    let tieBreak = "";
    if (tied.length > 1) {
      // Last tie-break: the item with an English Wikipedia article is the main item for the name.
      const withArticle = tied.filter((c) => hasEnwiki(c.id));
      if (withArticle.length === 1) {
        tied = withArticle;
        tieBreak = "; the only tied item with an English Wikipedia article";
      }
    }
    if (tied.length > 1) {
      return { candidate: null, evidence: "", weak: false, rejected: [...rejected, `ambiguous strong matches: ${tied.map(describe).join("; ")}`] };
    }
    const best = tied[0];
    const parts = [
      best.website ? `P856 matched ${hostOf(best.website)}` : "",
      best.cik ? `P5531 matched CIK ${padCik(best.cik)}` : "",
      best.ticker ? `P414/P249 matched ${best.ticker}` : "",
    ].filter(Boolean);
    const alsoNamed = candidates.filter((c) => c.nameMatch === "exact" && !strong.includes(c)).slice(0, 3);
    return {
      candidate: best,
      evidence: `${parts.join(", ")}${tieBreak}`,
      weak: false,
      rejected: [
        ...rejected,
        ...ranked.filter((c) => c !== best).map((c) => `also matched, ranked lower: ${describe(c)}`),
        ...alsoNamed.map((c) => `another item with one of our names (no strong match): ${describe(c)}`),
      ],
    };
  }
  const countryOk = (c: Candidate) => !org.hq?.country || c.countries.length === 0 || c.countries.includes(org.hq.country);
  const weak = candidates.filter((c) => c.nameMatch === "exact" && c.descFits && countryOk(c));
  if (weak.length === 1) {
    return {
      candidate: weak[0],
      evidence: `label or alias of "${weak[0].label}" matched our names and the description fits ("${weak[0].description}")`,
      weak: true,
      rejected,
    };
  }
  if (weak.length > 1) {
    return { candidate: null, evidence: "", weak: false, rejected: [...rejected, `several label matches with fitting descriptions: ${weak.map(describe).join("; ")}`] };
  }
  const named = candidates.filter((c) => c.nameMatch);
  const why = (c: Candidate) =>
    `${describe(c)}: ${c.nameMatch !== "exact" ? "name only partly matches" : !c.descFits ? "description does not clearly fit" : "country differs"}`;
  return {
    candidate: null,
    evidence: "",
    weak: false,
    rejected: [
      ...rejected,
      named.length > 0 ? `no strong match; name-only candidates: ${named.slice(0, 4).map(why).join("; ")}` : "no candidate with a matching name, website, CIK or ticker",
    ],
  };
}

function inceptionOf(entity: WdEntity): { date: string } | { problem: string } | null {
  const claims = currentClaims(entity, "P571");
  if (claims.length === 0) return null;
  const values = unique(
    claims.map((c) => {
      const v = c.mainsnak.datavalue?.value as { time?: string; precision?: number; calendarmodel?: string } | undefined;
      if (!v?.time || typeof v.precision !== "number") return "invalid";
      if (v.calendarmodel && !v.calendarmodel.endsWith("/Q1985727")) return "non-gregorian";
      const m = /^\+(\d{4})-(\d{2})-(\d{2})T/.exec(v.time);
      if (!m) return "invalid";
      if (v.precision >= 11) return `${m[1]}-${m[2]}-${m[3]}`;
      if (v.precision === 10) return `${m[1]}-${m[2]}`;
      if (v.precision === 9) return m[1];
      return "too-coarse";
    })
  );
  if (values.length > 1) return { problem: `several P571 values: ${values.join(", ")}` };
  if (!/^\d{4}(?:-\d{2}(?:-\d{2})?)?$/.test(values[0])) return { problem: `P571 not usable (${values[0]})` };
  return { date: values[0] };
}

const CITY_CLASS = /\b(city|town|metropolis|megacity|municipality|commune)\b/i;

/* --------------------------------------------------------- hugging face */

/** Hugging Face org names to check (AI labs and NVIDIA, Microsoft, Meta, Alibaba, Google only). */
const HF_CANDIDATES: Record<string, string[]> = {
  org_anthropic: ["Anthropic"],
  org_openai: ["openai"],
  org_xai: ["xai-org"],
  org_alphabet: ["google"],
  org_meta: ["meta-llama"],
  org_mistral_ai: ["mistralai"],
  org_deepseek: ["deepseek-ai"],
  org_moonshot_ai: ["moonshotai"],
  org_zai: ["zai-org"],
  org_minimax: ["MiniMaxAI"],
  org_alibaba: ["Qwen"],
  org_nvidia: ["nvidia"],
  org_microsoft: ["microsoft"],
};

/** GitHub org logins to check (60 requests per hour without a token, so only known public orgs). */
const GITHUB_CANDIDATES: Record<string, string> = {
  org_anthropic: "anthropics",
  org_openai: "openai",
  org_xai: "xai-org",
  org_mistral_ai: "mistralai",
  org_deepseek: "deepseek-ai",
  org_moonshot_ai: "MoonshotAI",
  org_zai: "zai-org",
  org_minimax: "MiniMax-AI",
  org_alibaba: "QwenLM",
  org_coreweave: "coreweave",
  org_nebius: "nebius",
  org_nvidia: "NVIDIA",
  org_meta: "meta-llama",
  org_alphabet: "google-deepmind",
  org_microsoft: "microsoft",
};

/**
 * Inception values we do not trust for our org, with the reason (found by reading the item).
 * The QID itself is still used.
 */
const FOUNDED_HOLD: Record<string, string> = {
  org_constellation_energy:
    "Q259195 also lists an NYSE CEG listing that ended in 2012 and two deprecated inceptions, " +
    "so its 1999 inception may describe the older Constellation Energy Group, not the company behind CIK 0001868275",
};

/** How our website and Wikidata's differ, or null when they are the same. */
function websiteDifference(ours: string | undefined, theirs: string): string | null {
  if (!ours) return `website missing in ours; Wikidata ${theirs}`;
  const a = ours.replace(/\/+$/, "");
  if (a === theirs) return null;
  const strip = (u: string) => u.replace(/^https?:\/\//i, "").replace(/^www\./i, "").toLowerCase();
  if (strip(a) === strip(theirs)) return `website differs only in http/https or www (keep ours): ours ${ours}, Wikidata ${theirs}`;
  return `website differs: ours ${ours}, Wikidata ${theirs}`;
}

/* ------------------------------------------------------------------ main */

interface OrgOut {
  wikidata?: string;
  founded?: { date: string; sourceId: string };
  hq?: { city?: string; country: string };
  website?: string;
  hfOrg?: string;
  githubOrg?: string;
  secCikVerified?: string[];
  evidence: string;
  differences: string[];
  notes: string[];
}

interface SourceOut {
  id: string;
  url: string;
  publisher: "Wikidata";
  type: "dataset";
  title: string;
  retrievedAt: string;
  license: "CC0";
}

async function main() {
  const { orgs, files } = loadOrgs();
  log(`Loaded ${orgs.length} orgs from ${files.join(", ")}.`);
  const out: Record<string, OrgOut> = {};
  for (const org of orgs) out[org.id] = { evidence: "", differences: [], notes: [] };
  const evidence = new Map<string, string[]>(orgs.map((o) => [o.id, []]));

  /* ---- Wikidata: candidates */
  const candidateIds = new Map<string, string[]>();
  for (const org of orgs) {
    const ids: string[] = [];
    for (const term of unique(orgNames(org))) for (const hit of await wdSearch(term)) ids.push(hit.id);
    for (const cik of org.ids.secCik) ids.push(...(await wdByCik(cik)));
    candidateIds.set(org.id, unique(ids));
  }
  /** Fetch what the evaluation needs: the items, their P31 classes, exchanges, countries and enwiki links. */
  const prepare = async (ids: string[]) => {
    await wdEntities(ids, FULL);
    const resolved = ids.map((id) => getEntity(id)?.id ?? id);
    const linked = (property: string) =>
      unique(resolved.flatMap((id) => anyClaims(getEntity(id), property).map(itemValue)).filter((v): v is string => !!v));
    await wdEntities(unique([...linked("P31"), ...linked("P414")]), "labels", {}, 50);
    await wdCountries(resolved);
    await wdEntities(resolved, SITELINKS.props, SITELINKS.extra, 50);
  };
  const evaluateAll = (org: Org) => {
    const seen = new Set<string>();
    const result: Candidate[] = [];
    for (const id of candidateIds.get(org.id) ?? []) {
      const entity = getEntity(id);
      // Search hits can be redirects; keep one candidate per resolved item.
      if (!entity || entity.missing || seen.has(entity.id)) continue;
      seen.add(entity.id);
      result.push(evaluate(org, entity));
    }
    return result;
  };
  await prepare(unique([...candidateIds.values()].flat()));
  log(`Wikidata: ${unique([...candidateIds.values()].flat()).length} candidate items for ${orgs.length} orgs.`);

  // Second pass for orgs without a strong match: look the website up as an exact P856 value.
  const secondPass = orgs.filter((org) => org.website && !pick(org, evaluateAll(org), hasEnwiki).candidate);
  for (const org of secondPass) {
    const found = await wdByWebsite(org.website!);
    if (found.length > 0) candidateIds.set(org.id, unique([...(candidateIds.get(org.id) ?? []), ...found]));
  }
  await prepare(unique(secondPass.flatMap((org) => candidateIds.get(org.id) ?? [])));

  const picks = new Map<string, { entity: WdEntity; pick: WikidataPick; candidates: Candidate[] }>();
  for (const org of orgs) {
    const candidates = evaluateAll(org);
    const result = pick(org, candidates, hasEnwiki);
    if (result.candidate) {
      picks.set(org.id, { entity: getEntity(result.candidate.id)!, pick: result, candidates });
    } else {
      out[org.id].notes.push(`Wikidata: not set (${result.rejected.join(" | ")})`);
    }
  }

  /* ---- Wikidata: HQ items, their countries and classes */
  const hqIds = new Map<string, string>();
  for (const [orgIdValue, { entity }] of picks) {
    const hq = unique(currentClaims(entity, "P159").map(itemValue).filter((v): v is string => !!v));
    if (hq.length === 1) hqIds.set(orgIdValue, hq[0]);
    else if (hq.length > 1) out[orgIdValue].notes.push(`Wikidata: several P159 headquarters (${hq.join(", ")}); city not set`);
  }
  await wdEntities([...hqIds.values()], FULL);
  await wdCountries([...hqIds.values()].map((id) => getEntity(id)?.id ?? id));
  const classIds = unique([...hqIds.values()].flatMap((id) => anyClaims(getEntity(id), "P31").map(itemValue)).filter((v): v is string => !!v));
  await wdEntities(classIds, "labels", {}, 50);

  const sources: SourceOut[] = [];
  for (const org of orgs) {
    const o = out[org.id];
    const ev = evidence.get(org.id)!;
    const chosen = picks.get(org.id);
    if (!chosen) continue;
    const { entity, pick: p, candidates } = chosen;
    const qid = entity.id;
    const sourceIdValue = `src_wikidata_${qid.toLowerCase()}`;
    o.wikidata = qid;
    ev.push(`Wikidata ${qid} "${label(entity)}": ${p.evidence}`);
    for (const r of p.rejected) o.notes.push(`Wikidata: ${r}`);
    if (p.weak) o.differences.push(`Wikidata ${qid} "${label(entity)}" was matched by name and description only; please confirm`);
    if (label(entity) && !sameName(org, label(entity)) && !containsName(org, label(entity))) {
      o.differences.push(`Wikidata label is "${label(entity)}" (ours "${org.name}")`);
    }
    sources.push({
      id: sourceIdValue,
      url: `https://www.wikidata.org/wiki/${qid}`,
      publisher: "Wikidata",
      type: "dataset",
      title: `Wikidata item ${qid} (${label(entity)})`,
      retrievedAt: (entityFetchedAt.get(qid) ?? new Date().toISOString()).slice(0, 10),
      license: "CC0",
    });

    const inception = inceptionOf(entity);
    if (FOUNDED_HOLD[org.id]) o.notes.push(`Wikidata: founded not set (${FOUNDED_HOLD[org.id]})`);
    else if (inception && "date" in inception) o.founded = { date: inception.date, sourceId: sourceIdValue };
    else if (inception) o.notes.push(`Wikidata: founded not set (${inception.problem})`);
    else o.notes.push("Wikidata: no P571 inception");

    // Country: the org's own P17, else the HQ item's P17.
    const hqId = hqIds.get(org.id);
    const hqEntity = hqId ? getEntity(hqId) : undefined;
    let countries = countryIdOf(entity);
    let countryFrom = "P17 of the item";
    if (countries.length !== 1 && hqEntity) {
      if (countries.length > 1) o.notes.push(`Wikidata: several P17 countries on the item (${countries.join(", ")}); using the HQ's country`);
      countries = countryIdOf(hqEntity);
      countryFrom = `P17 of the HQ ${hqId}`;
    }
    let country: string | null = null;
    if (countries.length === 1) {
      country = isoOf(countries[0]);
      if (!country) o.notes.push(`Wikidata: country ${countries[0]} has no single P297 code`);
    } else if (countries.length > 1) {
      o.notes.push(`Wikidata: several countries (${countries.join(", ")}); country not set`);
    } else {
      o.notes.push("Wikidata: no P17 country on the item or its HQ");
    }
    let city: string | null = null;
    if (hqEntity) {
      const classLabels = anyClaims(hqEntity, "P31")
        .map(itemValue)
        .filter((v): v is string => !!v)
        .map((id) => label(getLabelOnly(id)));
      const isCity = classLabels.some((l) => CITY_CLASS.test(l) && !/\bdistrict\b/i.test(l));
      if (isCity && label(hqEntity)) city = label(hqEntity);
      else o.notes.push(`Wikidata: P159 ${hqId} "${label(hqEntity)}" is not a city (${classLabels.filter(Boolean).slice(0, 4).join(", ") || "no class"}); city not set`);
    }
    if (country) {
      o.hq = { ...(city ? { city } : {}), country };
      ev.push(`country from ${countryFrom}`);
      if (org.hq?.country && org.hq.country !== country) o.differences.push(`hq.country differs: ours ${org.hq.country}, Wikidata ${country}`);
      if (org.hq?.city && city && org.hq.city !== city) o.differences.push(`hq.city differs: ours ${org.hq.city}, Wikidata ${city}`);
    }

    // Website: prefer the value on our domain, then https.
    const websites = currentClaims(entity, "P856").map(stringValue).filter((v): v is string => !!v);
    const ourHost = hostOf(org.website);
    const ordered = [...websites].sort(
      (a, b) => Number(hostOf(b) === ourHost) - Number(hostOf(a) === ourHost) || Number(/^https:/i.test(b)) - Number(/^https:/i.test(a))
    );
    const site = ordered[0]?.replace(/\/+$/, "");
    // A name-only pick whose website is also another candidate's website (a fund and its manager) is not trusted.
    const sharedWith = site
      ? candidates.filter((c) => c.id !== qid && anyClaims(getEntity(c.id), "P856").some((w) => hostOf(stringValue(w)) === hostOf(site)))
      : [];
    if (!site) {
      o.notes.push("Wikidata: no current P856 website");
    } else if (p.weak && sharedWith.length > 0) {
      o.notes.push(`Wikidata: website ${site} not set; the same site is P856 of ${sharedWith.map(describe).join("; ")}`);
    } else {
      o.website = site;
      const others = unique(websites.map((w) => w.replace(/\/+$/, ""))).filter((w) => w !== site);
      if (others.length > 0) o.notes.push(`Wikidata: other P856 values: ${others.join(", ")}`);
      const diff = websiteDifference(org.website, site);
      if (diff) o.differences.push(diff);
    }
  }

  /* ---- Hugging Face */
  for (const org of orgs) {
    const names = HF_CANDIDATES[org.id];
    if (!names) continue;
    for (const name of names) {
      const { status, data } = await huggingface.getJson<{ name?: string; fullname?: string; isVerified?: boolean; numModels?: number }>(
        `https://huggingface.co/api/organizations/${encodeURIComponent(name)}/overview`
      );
      if (status !== 200 || !data) {
        out[org.id].notes.push(`Hugging Face: ${name} returned HTTP ${status}; not set`);
        continue;
      }
      const fullname = data.fullname ?? "";
      const nameOk = data.name === name;
      const fullOk = sameName(org, fullname) || containsName(org, fullname) || sameName(org, fullname.replace(/_+$/, ""));
      if (nameOk && fullOk) {
        out[org.id].hfOrg = name;
        evidence.get(org.id)!.push(`HF ${name}: name exact, fullname "${fullname}"${data.isVerified ? ", verified" : ""}`);
        break;
      }
      out[org.id].notes.push(
        `Hugging Face: ${name} not set (returned name "${data.name}", fullname "${fullname}"; ${nameOk ? "fullname does not match the org" : "name is not exact"})`
      );
    }
  }

  /* ---- GitHub */
  let githubStopped: string | null = null;
  for (const org of orgs) {
    const login = GITHUB_CANDIDATES[org.id];
    if (!login) continue;
    if (githubStopped) {
      out[org.id].notes.push(`GitHub: ${login} not checked (${githubStopped})`);
      continue;
    }
    try {
      const { status, data } = await github.getJson<{ login?: string; name?: string | null; blog?: string | null; is_verified?: boolean; type?: string }>(
        `https://api.github.com/orgs/${encodeURIComponent(login)}`
      );
      if (status !== 200 || !data?.login) {
        out[org.id].notes.push(`GitHub: ${login} returned HTTP ${status}; not set`);
        continue;
      }
      const blogDomain = registrableDomain(hostOf(data.blog));
      const ourDomain = registrableDomain(hostOf(org.website));
      const domainOk = !!blogDomain && blogDomain === ourDomain;
      const nameOk = !!data.name && (sameName(org, data.name) || containsName(org, data.name));
      if (data.login.toLowerCase() === login.toLowerCase() && data.type === "Organization" && (domainOk || nameOk)) {
        out[org.id].githubOrg = data.login;
        const why = [nameOk ? `name "${data.name}"` : "", domainOk ? `blog ${data.blog}` : ""].filter(Boolean).join(", ");
        evidence.get(org.id)!.push(`GitHub ${data.login}: ${why}${data.is_verified ? ", verified" : ""}`);
      } else {
        out[org.id].notes.push(`GitHub: ${login} not set (name "${data.name ?? ""}", blog "${data.blog ?? ""}", type ${data.type})`);
      }
    } catch (error) {
      if (error instanceof HttpError && /rate limit/i.test(error.message)) {
        githubStopped = error.message;
        out[org.id].notes.push(`GitHub: ${login} not checked (${githubStopped})`);
      } else throw error;
    }
  }

  /* ---- SEC CIKs */
  for (const org of orgs) {
    const verified: string[] = [];
    for (const cik of org.ids.secCik) {
      try {
        const { data } = await sec.getJson<{ name?: string; formerNames?: Array<{ name: string }>; tickers?: string[]; exchanges?: string[] }>(
          submissionsUrl(cik)
        );
        // SEC conformed names can end with a state code ("Lambda, Inc./DE").
        const conformed = (name: string) => name.replace(/\s*\/[A-Z]{2}\/?\s*$/, "").trim();
        const secNames = [data.name ?? "", ...(data.formerNames ?? []).map((f) => f.name)].filter(Boolean).map(conformed);
        const current = data.name ?? "";
        if (sameName(org, conformed(current))) {
          verified.push(padCik(cik));
          evidence.get(org.id)!.push(`SEC CIK ${padCik(cik)}: "${current}"`);
        } else if (sameWordsAnyOrder(org, conformed(current))) {
          verified.push(padCik(cik));
          evidence.get(org.id)!.push(`SEC CIK ${padCik(cik)}: "${current}" (same words in another order)`);
        } else if (secNames.some((n) => sameName(org, n))) {
          verified.push(padCik(cik));
          evidence.get(org.id)!.push(`SEC CIK ${padCik(cik)}: current name "${current}", a former name matches`);
          out[org.id].differences.push(`SEC CIK ${padCik(cik)} current name is "${current}" (a former name matches ours)`);
        } else {
          out[org.id].differences.push(`SEC CIK ${padCik(cik)} name "${current}" does not match the org names; not verified`);
        }
        // Tickers: report ours that SEC does not list, and US listings SEC shows when we list none.
        const listed = (data.tickers ?? []).map((t, i) => ({ symbol: t.toUpperCase(), exchange: (data.exchanges ?? [])[i] ?? "" }));
        const usListed = listed.filter((t) => /^(nyse|nasdaq)$/i.test(t.exchange));
        const ours = org.ids.tickers.filter((t) => t.exchange === "NASDAQ" || t.exchange === "NYSE").map((t) => t.symbol);
        const missing = ours.filter((t) => !listed.some((l) => l.symbol === t));
        if (missing.length > 0) out[org.id].differences.push(`SEC CIK ${padCik(cik)} does not list our ticker(s) ${missing.join(", ")}`);
        if (ours.length === 0 && usListed.length > 0 && org.ids.tickers.length === 0) {
          out[org.id].differences.push(
            `SEC CIK ${padCik(cik)} lists ${usListed.map((t) => `${t.exchange}:${t.symbol}`).join(", ")}; ours has no ticker`
          );
        }
        const extra = listed.filter((l) => !ours.includes(l.symbol));
        if (ours.length > 0 && extra.length > 0) {
          out[org.id].notes.push(`SEC CIK ${padCik(cik)} also lists ${extra.map((t) => `${t.exchange || "?"}:${t.symbol}`).join(", ")}`);
        }
      } catch (error) {
        out[org.id].differences.push(`SEC CIK ${padCik(cik)} did not resolve: ${error instanceof Error ? error.message : String(error)}`);
      }
    }
    if (verified.length > 0) out[org.id].secCikVerified = verified;
  }

  for (const org of orgs) out[org.id].evidence = evidence.get(org.id)!.join("; ");

  /* ---- write */
  mkdirSync(OUT_DIR, { recursive: true });
  const orgsOut = Object.fromEntries(
    orgs.map((org) => {
      const o = out[org.id];
      return [
        org.id,
        {
          ...(o.wikidata ? { wikidata: o.wikidata } : {}),
          ...(o.founded ? { founded: o.founded } : {}),
          ...(o.hq ? { hq: o.hq } : {}),
          ...(o.website ? { website: o.website } : {}),
          ...(o.hfOrg ? { hfOrg: o.hfOrg } : {}),
          ...(o.githubOrg ? { githubOrg: o.githubOrg } : {}),
          ...(o.secCikVerified ? { secCikVerified: o.secCikVerified } : {}),
          evidence: o.evidence,
          differences: o.differences,
          notes: o.notes,
        },
      ];
    })
  );
  const generatedAt = new Date().toISOString();
  const json = { generatedAt, inputFiles: files, sources: sources.sort((a, b) => (a.id < b.id ? -1 : 1)), orgs: orgsOut };
  writeFileSync(join(OUT_DIR, "org-ids.json"), `${JSON.stringify(json, null, 2)}\n`);

  const count = (key: keyof OrgOut) => orgs.filter((o) => out[o.id][key] !== undefined).length;
  const withCiks = orgs.filter((o) => o.ids.secCik.length > 0);
  const md: string[] = [
    "# Capital v2 org identifiers",
    "",
    `Generated ${generatedAt} by \`bun scripts/capital/build-org-ids.ts\` from ${files.join(", ")}.`,
    "Only verified fields are in org-ids.json. Wikidata is CC0.",
    "",
    "## Counts",
    "",
    "| Field | Orgs |",
    "|---|---|",
    `| Orgs read | ${orgs.length} |`,
    `| Wikidata QID | ${count("wikidata")} |`,
    `| Founded (P571) | ${count("founded")} |`,
    `| HQ country | ${count("hq")} |`,
    `| HQ city | ${orgs.filter((o) => out[o.id].hq?.city).length} |`,
    `| Website (P856) | ${count("website")} |`,
    `| Hugging Face org | ${count("hfOrg")} |`,
    `| GitHub org | ${count("githubOrg")} |`,
    `| Orgs with all listed CIKs verified | ${withCiks.filter((o) => out[o.id].secCikVerified?.length === o.ids.secCik.length).length} of ${withCiks.length} |`,
    "",
    "## Differences from our org files",
    "",
  ];
  const withDiffs = orgs.filter((o) => out[o.id].differences.length > 0);
  if (withDiffs.length === 0) md.push("None.");
  for (const org of withDiffs) for (const d of out[org.id].differences) md.push(`- ${org.id}: ${d}`);
  md.push("", "## Not set or not verified", "");
  // The full notes (rejected candidates, HQ items that are not cities, extra SEC tickers) stay in org-ids.json.
  const important = /^(Wikidata: not set|Wikidata: founded not set|Wikidata: website .* not set|Hugging Face|GitHub)/;
  for (const org of orgs) for (const n of out[org.id].notes.filter((note) => important.test(note))) md.push(`- ${org.id}: ${n}`);
  md.push("", "Other notes (rejected candidates, HQ items that are not cities, extra SEC tickers) are in org-ids.json.");
  md.push("", "## Per org", "", "| Org | QID | Founded | HQ | Website | HF | GitHub | CIKs verified |", "|---|---|---|---|---|---|---|---|");
  for (const org of orgs) {
    const o = out[org.id];
    const hq = o.hq ? [o.hq.city, o.hq.country].filter(Boolean).join(", ") : "";
    const cells = [org.id, o.wikidata, o.founded?.date, hq, o.website, o.hfOrg, o.githubOrg, (o.secCikVerified ?? []).join(", ")];
    md.push(`| ${cells.map((c) => c ?? "").join(" | ")} |`);
  }
  writeFileSync(join(OUT_DIR, "org-ids.md"), `${md.join("\n")}\n`);

  const stats = { wikidata: wikidata.stats, huggingface: huggingface.stats, github: github.stats, sec: sec.stats };
  log(`Done. ${Object.entries(stats).map(([name, s]) => `${name} ${JSON.stringify(s)}`).join(", ")}`);
  log(`Wrote ${join(OUT_DIR, "org-ids.json")} and org-ids.md: ${count("wikidata")}/${orgs.length} QIDs.`);
}

await main();
