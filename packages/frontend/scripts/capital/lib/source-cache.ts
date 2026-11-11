/**
 * Fetch and cache the documents that Capital ledger sources cite: company newsroom pages,
 * SEC filings and press pages. Used by scripts/capital/verify-quotes.ts and by hand while
 * researching events.
 *
 * Network rules:
 * - SEC hosts (sec.gov and its subdomains) get the declared User-Agent
 *   `Tokscale research i@junho.io` and at most 2 request starts per second from this process.
 * - Other hosts get `Mozilla/5.0 (compatible; TokscaleResearch/1.0; +mailto:i@junho.io)` and at
 *   most 1 request start per second per host.
 * - Every request times out after 30 seconds. HTTP 408, 425, 429, 5xx and network errors are
 *   retried (4 attempts in total) with exponential backoff; Retry-After is honored up to 30 s.
 * - A blocked page (403, bot check) is recorded as it is. There is no workaround.
 *
 * Cache: `.stealth-cache/capital/sources/` (gitignored). For each URL, key = the first 16 hex
 * digits of sha256(url). The body is saved as `<key>.body` and its manifest entry as `<key>.json`
 * (url, fetchedAt, status, sha256, bytes, contentType, finalUrl). One small file per URL means
 * concurrent runs never overwrite each other's entries. Retryable failures are never saved.
 * Copies that other research scripts saved in the same directory (`<24 hex>.json` + `.html`) and
 * SEC documents that scripts/capital/lib/sec-client.ts saved under `.stealth-cache/capital/sec/` are
 * read (read-only, SHA-256 checked) instead of being fetched again.
 *
 * Modes:
 * - "cache" (default): use the saved response when there is one; fetch only what is missing.
 * - "refresh": fetch again and replace the saved response.
 * - "offline": never touch the network; a missing response is an error.
 *
 * Text helpers: `visibleText(html)` drops scripts, styles and tags and decodes entities.
 * `normalizeForMatch(text)` removes differences in whitespace and quote style only.
 *
 * CLI (fills the cache and writes the extracted text to `sources/text/<key>.txt` for reading):
 *   bun scripts/capital/lib/source-cache.ts <url> [<url> ...] [--refresh | --offline]
 */
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, readdirSync, renameSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

export const SEC_USER_AGENT = "Tokscale research i@junho.io";
export const WEB_USER_AGENT = "Mozilla/5.0 (compatible; TokscaleResearch/1.0; +mailto:i@junho.io)";
/** 500 ms between SEC request starts = at most 2 requests per second. */
export const SEC_MIN_INTERVAL_MS = 500;
/** 1 s between request starts to the same non-SEC host. */
export const WEB_MIN_INTERVAL_MS = 1_000;
export const TIMEOUT_MS = 30_000;
const MAX_ATTEMPTS = 4;
const MAX_BACKOFF_MS = 30_000;
const RETRY_STATUSES = new Set([408, 425, 429, 500, 502, 503, 504]);

export const SOURCE_CACHE_DIR = fileURLToPath(new URL("../../../.stealth-cache/capital/sources/", import.meta.url));
/** Cache written by sec-client.ts. Read-only here. */
export const SEC_CLIENT_CACHE_DIR = fileURLToPath(new URL("../../../.stealth-cache/capital/sec/", import.meta.url));

export type CacheMode = "cache" | "refresh" | "offline";

export interface SourceCacheEntry {
  url: string;
  /** First 16 hex digits of sha256(url). */
  key: string;
  /** Body file name inside the cache directory. */
  file: string;
  /** ISO timestamp (UTC) of the network fetch. */
  fetchedAt: string;
  status: number;
  /** SHA-256 of the saved body, hex. */
  sha256: string;
  bytes: number;
  contentType?: string;
  /** URL after redirects, when it differs from `url`. */
  finalUrl?: string;
}

export interface SourceResponse {
  url: string;
  status: number;
  body: string;
  contentType: string | null;
  fetchedAt: string;
  sha256: string;
  /** "network", "cache" (sources/) or "sec-cache" (sec-client's cache). */
  origin: "network" | "cache" | "sec-cache";
  file: string;
}

export class SourceFetchError extends Error {
  constructor(
    readonly url: string,
    message: string
  ) {
    super(message);
    this.name = "SourceFetchError";
  }
}

export class CacheMissError extends Error {
  constructor(readonly url: string) {
    super(`not in the source cache (offline mode): ${url}`);
    this.name = "CacheMissError";
  }
}

/* -------------------------------------------------------------- pacing */

const delay = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));

export function isSecHost(hostname: string): boolean {
  const host = hostname.toLowerCase();
  return host === "sec.gov" || host.endsWith(".sec.gov");
}

interface Lane {
  gate: Promise<void>;
  lastStartMs: number;
}
const lanes = new Map<string, Lane>();

/** Resolves when a request to this URL may start. SEC shares one lane; other hosts get one lane each. */
function nextSlot(url: string): Promise<void> {
  const host = new URL(url).hostname.toLowerCase();
  const sec = isSecHost(host);
  const name = sec ? "sec" : host;
  const interval = sec ? SEC_MIN_INTERVAL_MS : WEB_MIN_INTERVAL_MS;
  let lane = lanes.get(name);
  if (!lane) {
    lane = { gate: Promise.resolve(), lastStartMs: 0 };
    lanes.set(name, lane);
  }
  const current = lane;
  const turn = current.gate.then(async () => {
    const wait = current.lastStartMs + interval - Date.now();
    if (wait > 0) await delay(wait);
    current.lastStartMs = Date.now();
  });
  current.gate = turn.catch(() => undefined);
  return turn;
}

function retryAfterMs(header: string | null): number | null {
  if (!header) return null;
  const seconds = Number(header);
  if (Number.isFinite(seconds) && seconds >= 0) return Math.min(seconds * 1000, MAX_BACKOFF_MS);
  const at = Date.parse(header);
  return Number.isFinite(at) ? Math.min(Math.max(at - Date.now(), 0), MAX_BACKOFF_MS) : null;
}

/* --------------------------------------------------------------- cache */

const sha256Hex = (data: Uint8Array | string) => createHash("sha256").update(data).digest("hex");

export const sourceCacheKey = (url: string) => sha256Hex(url).slice(0, 16);

function writeAtomic(path: string, data: string | Uint8Array): void {
  const tmp = `${path}.${process.pid}.${Date.now()}.tmp`;
  writeFileSync(tmp, data);
  renameSync(tmp, path);
}

/** Decode a body with the charset from the Content-Type header (UTF-8 when absent or unknown). */
export function decodeBody(bytes: Uint8Array, contentType: string | null | undefined): string {
  const charset = /charset=["']?([\w.:-]+)/i.exec(contentType ?? "")?.[1]?.toLowerCase();
  if (charset && charset !== "utf-8" && charset !== "utf8") {
    try {
      return new TextDecoder(charset).decode(bytes);
    } catch {
      // Unknown label: fall back to UTF-8.
    }
  }
  return new TextDecoder("utf-8").decode(bytes);
}

function readEntry(dir: string, key: string): SourceCacheEntry | null {
  const path = join(dir, `${key}.json`);
  if (!existsSync(path)) return null;
  try {
    return JSON.parse(readFileSync(path, "utf8")) as SourceCacheEntry;
  } catch {
    return null;
  }
}

/** Every manifest entry in the source cache, sorted by URL. */
export function listCacheEntries(dir: string = SOURCE_CACHE_DIR): SourceCacheEntry[] {
  if (!existsSync(dir)) return [];
  return readdirSync(dir)
    .filter((name) => /^[0-9a-f]{16}\.json$/.test(name))
    .map((name) => readEntry(dir, name.slice(0, 16)))
    .filter((entry): entry is SourceCacheEntry => entry !== null)
    .sort((a, b) => (a.url < b.url ? -1 : a.url > b.url ? 1 : 0));
}

function fromSourceCache(url: string, dir: string): SourceResponse | null {
  const key = sourceCacheKey(url);
  const entry = readEntry(dir, key);
  if (!entry || entry.url !== url) return null;
  const path = join(dir, entry.file);
  if (!existsSync(path)) return null;
  const bytes = new Uint8Array(readFileSync(path));
  const sha256 = sha256Hex(bytes);
  if (sha256 !== entry.sha256) return null;
  return {
    url,
    status: entry.status,
    body: decodeBody(bytes, entry.contentType),
    contentType: entry.contentType ?? null,
    fetchedAt: entry.fetchedAt,
    sha256,
    origin: "cache",
    file: path,
  };
}

/**
 * Entries that other Capital research scripts saved in the same directory with a longer key
 * (`<first 24 hex digits of sha256(url)>.json` next to a `.html` or `.body` file). Read-only here.
 */
const LEGACY_KEY_LENGTHS = [24];

function fromLegacyCache(url: string, dir: string): SourceResponse | null {
  for (const length of LEGACY_KEY_LENGTHS) {
    const key = sha256Hex(url).slice(0, length);
    let entry: Partial<SourceCacheEntry> | null = null;
    try {
      const path = join(dir, `${key}.json`);
      if (existsSync(path)) entry = JSON.parse(readFileSync(path, "utf8")) as Partial<SourceCacheEntry>;
    } catch {
      entry = null;
    }
    if (!entry || entry.url !== url || typeof entry.sha256 !== "string" || typeof entry.status !== "number") continue;
    const candidates = [entry.file, `${key}.html`, `${key}.body`, `${key}.xml`, `${key}.txt`].filter((f): f is string => Boolean(f));
    for (const name of candidates) {
      const path = join(dir, name);
      if (!existsSync(path)) continue;
      const bytes = new Uint8Array(readFileSync(path));
      if (sha256Hex(bytes) !== entry.sha256) continue;
      return {
        url,
        status: entry.status,
        body: decodeBody(bytes, entry.contentType),
        contentType: entry.contentType ?? null,
        fetchedAt: entry.fetchedAt ?? "",
        sha256: entry.sha256,
        origin: "cache",
        file: path,
      };
    }
  }
  return null;
}

interface SecClientEntry {
  url: string;
  file: string;
  fetchedAt: string;
  status: number;
  sha256: string;
  contentType?: string;
}

/** A 200 response that sec-client.ts saved, if its bytes still match its manifest. */
function fromSecClientCache(url: string, dir: string): SourceResponse | null {
  if (!isSecHost(new URL(url).hostname)) return null;
  const manifestPath = join(dir, "manifest.json");
  if (!existsSync(manifestPath)) return null;
  let entry: SecClientEntry | undefined;
  try {
    entry = (JSON.parse(readFileSync(manifestPath, "utf8")) as { entries?: Record<string, SecClientEntry> }).entries?.[url];
  } catch {
    return null;
  }
  if (!entry || entry.status !== 200) return null;
  const path = join(dir, entry.file);
  if (!existsSync(path)) return null;
  const bytes = new Uint8Array(readFileSync(path));
  const sha256 = sha256Hex(bytes);
  if (sha256 !== entry.sha256) return null;
  return {
    url,
    status: entry.status,
    body: decodeBody(bytes, entry.contentType),
    contentType: entry.contentType ?? null,
    fetchedAt: entry.fetchedAt,
    sha256,
    origin: "sec-cache",
    file: path,
  };
}

function record(dir: string, url: string, res: { status: number; bytes: Uint8Array; contentType: string | null; finalUrl: string }, fetchedAt: string): SourceCacheEntry {
  mkdirSync(dir, { recursive: true });
  const key = sourceCacheKey(url);
  const file = `${key}.body`;
  writeAtomic(join(dir, file), res.bytes);
  const entry: SourceCacheEntry = {
    url,
    key,
    file,
    fetchedAt,
    status: res.status,
    sha256: sha256Hex(res.bytes),
    bytes: res.bytes.byteLength,
    ...(res.contentType ? { contentType: res.contentType } : {}),
    ...(res.finalUrl && res.finalUrl !== url ? { finalUrl: res.finalUrl } : {}),
  };
  writeAtomic(join(dir, `${key}.json`), `${JSON.stringify(entry, null, 2)}\n`);
  return entry;
}

async function fetchNetwork(url: string, log: (line: string) => void): Promise<{ status: number; bytes: Uint8Array; contentType: string | null; finalUrl: string }> {
  const sec = isSecHost(new URL(url).hostname);
  let problem = "";
  for (let attempt = 1; attempt <= MAX_ATTEMPTS; attempt++) {
    await nextSlot(url);
    let waitMs: number | null = null;
    try {
      const res = await fetch(url, {
        headers: {
          "User-Agent": sec ? SEC_USER_AGENT : WEB_USER_AGENT,
          Accept: "text/html,application/xhtml+xml,application/xml;q=0.9,application/json;q=0.8,*/*;q=0.5",
          "Accept-Language": "en",
        },
        signal: AbortSignal.timeout(TIMEOUT_MS),
        redirect: "follow",
      });
      const bytes = new Uint8Array(await res.arrayBuffer());
      log(`fetch  ${res.status} ${url} (${bytes.byteLength} bytes, attempt ${attempt})`);
      if (!RETRY_STATUSES.has(res.status)) {
        return { status: res.status, bytes, contentType: res.headers.get("content-type"), finalUrl: res.url || url };
      }
      problem = `HTTP ${res.status}`;
      waitMs = retryAfterMs(res.headers.get("retry-after"));
    } catch (error) {
      problem = error instanceof Error ? `${error.name}: ${error.message}` : String(error);
      log(`fetch  failed ${url} (attempt ${attempt}): ${problem}`);
    }
    if (attempt < MAX_ATTEMPTS) await delay(waitMs ?? Math.min(1000 * 2 ** (attempt - 1), MAX_BACKOFF_MS));
  }
  throw new SourceFetchError(url, `request failed after ${MAX_ATTEMPTS} attempts (${problem}): ${url}`);
}

export interface FetchSourceOptions {
  mode?: CacheMode;
  cacheDir?: string;
  secClientCacheDir?: string;
  log?: (line: string) => void;
}

/**
 * The document at `url`, from the cache or the network depending on the mode. Any final HTTP status
 * is returned (callers decide what a 403 or 404 means); only retryable failures throw.
 */
export async function fetchSource(url: string, options: FetchSourceOptions = {}): Promise<SourceResponse> {
  const mode = options.mode ?? "cache";
  const dir = options.cacheDir ?? SOURCE_CACHE_DIR;
  const log = options.log ?? (() => undefined);
  if (mode !== "refresh") {
    const cached =
      fromSourceCache(url, dir) ?? fromLegacyCache(url, dir) ?? fromSecClientCache(url, options.secClientCacheDir ?? SEC_CLIENT_CACHE_DIR);
    if (cached) return cached;
    if (mode === "offline") throw new CacheMissError(url);
  }
  const fetchedAt = new Date().toISOString();
  const res = await fetchNetwork(url, log);
  const entry = record(dir, url, res, fetchedAt);
  return {
    url,
    status: res.status,
    body: decodeBody(res.bytes, res.contentType),
    contentType: res.contentType,
    fetchedAt,
    sha256: entry.sha256,
    origin: "network",
    file: join(dir, entry.file),
  };
}

/* ---------------------------------------------------------------- text */

// HTML 4 Latin-1 entities, code points 160..255 in order.
const LATIN1_ENTITIES =
  "nbsp iexcl cent pound curren yen brvbar sect uml copy ordf laquo not shy reg macr deg plusmn sup2 sup3 acute micro para middot cedil sup1 ordm raquo frac14 frac12 frac34 iquest Agrave Aacute Acirc Atilde Auml Aring AElig Ccedil Egrave Eacute Ecirc Euml Igrave Iacute Icirc Iuml ETH Ntilde Ograve Oacute Ocirc Otilde Ouml times Oslash Ugrave Uacute Ucirc Uuml Yacute THORN szlig agrave aacute acirc atilde auml aring aelig ccedil egrave eacute ecirc euml igrave iacute icirc iuml eth ntilde ograve oacute ocirc otilde ouml divide oslash ugrave uacute ucirc uuml yacute thorn yuml".split(
    " "
  );

const NAMED_ENTITIES: Record<string, number> = {
  ...Object.fromEntries(LATIN1_ENTITIES.map((name, i) => [name, 160 + i])),
  amp: 38, lt: 60, gt: 62, quot: 34, apos: 39, Tab: 9, NewLine: 10,
  OElig: 338, oelig: 339, Scaron: 352, scaron: 353, Yuml: 376, fnof: 402, circ: 710, tilde: 732,
  ensp: 8194, emsp: 8195, thinsp: 8201, hairsp: 8202, zwsp: 8203, zwnj: 8204, zwj: 8205, lrm: 8206, rlm: 8207,
  hyphen: 8208, dash: 8208, ndash: 8211, mdash: 8212, horbar: 8213, lsquo: 8216, rsquo: 8217, rsquor: 8217, sbquo: 8218,
  ldquo: 8220, rdquo: 8221, rdquor: 8221, bdquo: 8222, dagger: 8224, Dagger: 8225, bull: 8226, bullet: 8226,
  nldr: 8229, hellip: 8230, mldr: 8230, permil: 8240, prime: 8242, Prime: 8243, lsaquo: 8249, rsaquo: 8250,
  oline: 8254, frasl: 8260, NoBreak: 8288, euro: 8364, trade: 8482, TRADE: 8482, larr: 8592, rarr: 8594,
  uarr: 8593, darr: 8595, harr: 8596, minus: 8722, le: 8804, ge: 8805, ne: 8800, asymp: 8776, infin: 8734,
  check: 10003, nbhy: 8209, numsp: 8199, puncsp: 8200, MediumSpace: 8287, ThinSpace: 8201, nbspace: 160,
};

// Numeric references 128..159 mean Windows-1252 characters (HTML parsing rules).
const CP1252: Record<number, number> = {
  128: 8364, 130: 8218, 131: 402, 132: 8222, 133: 8230, 134: 8224, 135: 8225, 136: 710, 137: 8240, 138: 352,
  139: 8249, 140: 338, 142: 381, 145: 8216, 146: 8217, 147: 8220, 148: 8221, 149: 8226, 150: 8211, 151: 8212,
  152: 732, 153: 8482, 154: 353, 155: 8250, 156: 339, 158: 382, 159: 376,
};

function fromCodePoint(code: number): string {
  const mapped = CP1252[code] ?? code;
  if (!Number.isFinite(mapped) || mapped <= 0 || mapped > 0x10ffff || (mapped >= 0xd800 && mapped <= 0xdfff)) return "\uFFFD";
  return String.fromCodePoint(mapped);
}

export function decodeEntities(text: string): string {
  return text.replace(/&(?:#(\d{1,7})|#[xX]([0-9a-fA-F]{1,6})|([A-Za-z][A-Za-z0-9]{1,31}));/g, (match, dec, hex, name) => {
    if (dec) return fromCodePoint(Number.parseInt(dec, 10));
    if (hex) return fromCodePoint(Number.parseInt(hex, 16));
    const code = NAMED_ENTITIES[name as string];
    return code === undefined ? match : String.fromCodePoint(code);
  });
}

/** Elements whose content is never visible text. */
const HIDDEN_ELEMENTS = /<(script|style|noscript|template|svg|iframe|head|object|canvas)\b(?:[^>"']|"[^"]*"|'[^']*')*>[\s\S]*?<\/\1\s*>/gi;
/** Inline elements: removing their tags must not add a space ("<b>$</b>65" is "$65"). */
const INLINE_TAGS = new Set([
  "a", "abbr", "b", "bdi", "bdo", "cite", "code", "data", "dfn", "em", "font", "i", "img", "kbd", "mark", "nobr", "q",
  "s", "samp", "small", "span", "strong", "sub", "sup", "time", "tt", "u", "var", "wbr", "del", "ins", "big", "strike",
]);
const TAG = /<\/?([A-Za-z][\w:.-]*)(?:[^>"']|"[^"]*"|'[^']*')*\/?>/g;

/** The text a reader sees: no scripts, styles, comments or tags; entities decoded. Not normalized. */
export function visibleText(html: string): string {
  return decodeEntities(
    html
      .replace(/<!--[\s\S]*?-->/g, " ")
      .replace(/<!\[CDATA\[([\s\S]*?)\]\]>/g, "$1")
      .replace(HIDDEN_ELEMENTS, " ")
      .replace(/<![^>]*>/g, " ")
      .replace(/<\?[\s\S]*?\?>/g, " ")
      .replace(TAG, (_match, name: string) => (INLINE_TAGS.has(name.toLowerCase()) ? "" : " "))
  );
}

/**
 * Make two strings comparable when they differ only in whitespace or quote style:
 * removes invisible characters, straightens curly quotes, collapses whitespace, and removes
 * spaces before closing punctuation, after opening brackets (link and tag boundaries add them)
 * and around en and em dashes ("partners — Micron" and "partners—Micron" compare equal).
 */
export function normalizeForMatch(text: string): string {
  return text
    .normalize("NFC")
    .replace(/[\u00AD\u200B\u200C\u200D\u2060\uFEFF]/g, "")
    .replace(/[\u2018\u2019\u201A\u201B\u2032\u2035]/g, "'")
    .replace(/[\u201C\u201D\u201E\u201F\u2033\u2036]/g, '"')
    .replace(/[\s\u00A0\u1680\u2000-\u200A\u2028\u2029\u202F\u205F\u3000]+/g, " ")
    .replace(/ (?=[.,;:!?%)\]}])/g, "")
    .replace(/([(\[{]) /g, "$1")
    .replace(/ ?([\u2013\u2014]) ?/g, "$1")
    .trim();
}

/* ----------------------------------------------------------------- CLI */

if (import.meta.main) {
  const args = process.argv.slice(2);
  const mode: CacheMode = args.includes("--refresh") ? "refresh" : args.includes("--offline") ? "offline" : "cache";
  const urls = args.filter((arg) => !arg.startsWith("--"));
  if (urls.length === 0) {
    console.error("usage: bun scripts/capital/lib/source-cache.ts <url> [<url> ...] [--refresh | --offline]");
    process.exit(2);
  }
  const textDir = join(SOURCE_CACHE_DIR, "text");
  mkdirSync(textDir, { recursive: true });
  let failed = 0;
  for (const url of urls) {
    try {
      const res = await fetchSource(url, { mode, log: (line) => console.error(line) });
      const text = normalizeForMatch(visibleText(res.body));
      const textFile = join(textDir, `${sourceCacheKey(url)}.txt`);
      writeAtomic(textFile, `${text}\n`);
      console.log(JSON.stringify({ url, status: res.status, origin: res.origin, fetchedAt: res.fetchedAt, sha256: res.sha256, file: res.file, textFile, textChars: text.length }));
      if (res.status !== 200) failed++;
    } catch (error) {
      failed++;
      console.log(JSON.stringify({ url, error: error instanceof Error ? error.message : String(error) }));
    }
  }
  process.exit(failed > 0 ? 1 : 0);
}
