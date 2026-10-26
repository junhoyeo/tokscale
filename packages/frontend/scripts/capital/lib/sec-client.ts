/**
 * SEC EDGAR client for the Capital build scripts.
 *
 * SEC fair-access rules this client follows:
 * - Every request sends the declared User-Agent `Tokscale research i@junho.io`.
 * - At most 2 requests per second from this process. All SecClient instances share one gate.
 * - Every request times out after 30 seconds. HTTP 429, 5xx and network errors are retried
 *   (4 attempts in total) with exponential backoff; a Retry-After header is honored up to 30 seconds.
 *
 * Cache: every response is saved under `.stealth-cache/capital/sec/` (gitignored) and listed in
 * `manifest.json` with its URL, fetch time, HTTP status and SHA-256. Only 200 and 404 responses are
 * saved; other errors are never cached. Several scripts share this folder, so each entry is also
 * written next to its file as `<file>.meta.json`; an entry that a concurrent writer dropped from
 * manifest.json is read from there and put back. Modes:
 * - "cache" (default): use the saved response when there is one; fetch only what is missing.
 * - "refresh": fetch again and replace the saved response (use it to pick up new filings).
 * - "replay": offline. Never touches the network; a missing response is an error.
 *
 * Usage:
 *   const sec = new SecClient({ mode: "cache" });
 *   const { data, fetchedAt } = await sec.getJson(companyFactsUrl("789019"));
 */
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

export const SEC_USER_AGENT = "Tokscale research i@junho.io";
/** 500 ms between request starts = at most 2 requests per second. */
export const SEC_MIN_INTERVAL_MS = 500;
export const SEC_TIMEOUT_MS = 30_000;
const MAX_ATTEMPTS = 4;
const MAX_BACKOFF_MS = 30_000;
const RETRY_STATUSES = new Set([429, 500, 502, 503, 504]);
/** Statuses worth saving: a 404 is a stable answer ("no XBRL data"); anything else may be transient. */
const CACHEABLE_STATUSES = new Set([200, 404]);

export const DEFAULT_SEC_CACHE_DIR = fileURLToPath(new URL("../../../.stealth-cache/capital/sec/", import.meta.url));

export type SecMode = "cache" | "refresh" | "replay";

export interface SecManifestEntry {
  url: string;
  /** File name inside the cache directory. */
  file: string;
  /** ISO timestamp (UTC) of the network fetch. */
  fetchedAt: string;
  status: number;
  /** SHA-256 of the saved body, hex. */
  sha256: string;
  bytes: number;
  contentType?: string;
}

interface SecManifest {
  version: 1;
  entries: Record<string, SecManifestEntry>;
}

export interface SecResponse {
  url: string;
  status: number;
  body: string;
  fetchedAt: string;
  sha256: string;
  fromCache: boolean;
}

export class SecHttpError extends Error {
  constructor(
    readonly url: string,
    readonly status: number,
    message: string
  ) {
    super(message);
    this.name = "SecHttpError";
  }
}

export class SecReplayMissError extends Error {
  constructor(readonly url: string) {
    super(`not in the SEC cache (replay mode is offline): ${url}`);
    this.name = "SecReplayMissError";
  }
}

/* ------------------------------------------------------------- pacing */

// One gate for the whole process: SEC limits requests per client, not per object.
let gate: Promise<void> = Promise.resolve();
let lastStartMs = 0;

const delay = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));

/** Resolves when this request may start. Request starts are at least SEC_MIN_INTERVAL_MS apart. */
function nextSlot(): Promise<void> {
  const turn = gate.then(async () => {
    const wait = lastStartMs + SEC_MIN_INTERVAL_MS - Date.now();
    if (wait > 0) await delay(wait);
    lastStartMs = Date.now();
  });
  gate = turn.catch(() => undefined);
  return turn;
}

/* -------------------------------------------------------------- helpers */

export function padCik(cik: string | number): string {
  const digits = String(cik).replace(/^CIK/i, "").trim();
  if (!/^\d{1,10}$/.test(digits)) throw new Error(`not a CIK: ${cik}`);
  return digits.padStart(10, "0");
}

export const companyFactsUrl = (cik: string | number) =>
  `https://data.sec.gov/api/xbrl/companyfacts/CIK${padCik(cik)}.json`;

export const submissionsUrl = (cik: string | number) => `https://data.sec.gov/submissions/CIK${padCik(cik)}.json`;

const sha256Hex = (bytes: Uint8Array) => createHash("sha256").update(bytes).digest("hex");

/** A readable, stable file name for one URL: host and path, plus a short hash when there is a query. */
export function cacheFileName(url: string): string {
  const u = new URL(url);
  const base = `${u.hostname}${u.pathname}`.replace(/[^A-Za-z0-9._-]+/g, "_").replace(/_+$/, "");
  const query = u.search ? `__${createHash("sha256").update(u.search).digest("hex").slice(0, 12)}` : "";
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

/* --------------------------------------------------------------- client */

export interface SecClientOptions {
  mode?: SecMode;
  cacheDir?: string;
  /** Called once per network attempt and cache hit (for progress logs). */
  log?: (line: string) => void;
}

export class SecClient {
  readonly mode: SecMode;
  readonly cacheDir: string;
  readonly stats = { network: 0, cache: 0, retries: 0 };
  private readonly log: (line: string) => void;

  constructor(options: SecClientOptions = {}) {
    this.mode = options.mode ?? "cache";
    this.cacheDir = options.cacheDir ?? DEFAULT_SEC_CACHE_DIR;
    this.log = options.log ?? (() => undefined);
  }

  get manifestPath(): string {
    return join(this.cacheDir, "manifest.json");
  }

  readManifest(): SecManifest {
    if (!existsSync(this.manifestPath)) return { version: 1, entries: {} };
    const parsed = JSON.parse(readFileSync(this.manifestPath, "utf8")) as SecManifest;
    return { version: 1, entries: parsed.entries ?? {} };
  }

  private sidecarPath(file: string): string {
    return join(this.cacheDir, `${file}.meta.json`);
  }

  /** Adds or replaces one entry in manifest.json (re-read right before writing to keep other writers' entries). */
  private mergeIntoManifest(entry: SecManifestEntry): void {
    const manifest = this.readManifest();
    manifest.entries[entry.url] = entry;
    const sorted = Object.fromEntries(Object.entries(manifest.entries).sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0)));
    writeAtomic(this.manifestPath, `${JSON.stringify({ version: 1, entries: sorted }, null, 2)}\n`);
  }

  /** The manifest entry for a URL; falls back to the entry saved next to the file. */
  private entryFor(url: string): SecManifestEntry | null {
    const listed = this.readManifest().entries[url];
    if (listed) return listed;
    const sidecar = this.sidecarPath(cacheFileName(url));
    if (!existsSync(sidecar)) return null;
    try {
      const entry = JSON.parse(readFileSync(sidecar, "utf8")) as SecManifestEntry;
      if (entry.url !== url) return null;
      this.mergeIntoManifest(entry); // put back an entry a concurrent writer dropped
      return entry;
    } catch {
      return null;
    }
  }

  /** The saved response for a URL, or null when it is missing or its bytes do not match the manifest. */
  private fromCache(url: string): SecResponse | null {
    const entry = this.entryFor(url);
    if (!entry) return null;
    const path = join(this.cacheDir, entry.file);
    if (!existsSync(path)) return null;
    const bytes = new Uint8Array(readFileSync(path));
    const sha256 = sha256Hex(bytes);
    if (sha256 !== entry.sha256) {
      this.log(`cache  ${url}: SHA-256 does not match the manifest; ignoring the saved copy`);
      return null;
    }
    // Entries saved before sidecars existed get one now, so a dropped manifest entry can be recovered.
    if (!existsSync(this.sidecarPath(entry.file))) writeAtomic(this.sidecarPath(entry.file), `${JSON.stringify(entry, null, 2)}\n`);
    return { url, status: entry.status, body: new TextDecoder().decode(bytes), fetchedAt: entry.fetchedAt, sha256, fromCache: true };
  }

  private record(url: string, status: number, bytes: Uint8Array, fetchedAt: string, contentType: string | null): string {
    mkdirSync(this.cacheDir, { recursive: true });
    const file = cacheFileName(url);
    writeAtomic(join(this.cacheDir, file), bytes);
    const sha256 = sha256Hex(bytes);
    const entry: SecManifestEntry = {
      url,
      file,
      fetchedAt,
      status,
      sha256,
      bytes: bytes.byteLength,
      ...(contentType ? { contentType } : {}),
    };
    writeAtomic(this.sidecarPath(file), `${JSON.stringify(entry, null, 2)}\n`);
    this.mergeIntoManifest(entry);
    return sha256;
  }

  private async fetchNetwork(url: string): Promise<{ status: number; bytes: Uint8Array; contentType: string | null }> {
    let problem = "";
    for (let attempt = 1; attempt <= MAX_ATTEMPTS; attempt++) {
      await nextSlot();
      this.stats.network++;
      let waitMs: number | null = null;
      try {
        const res = await fetch(url, {
          headers: { "User-Agent": SEC_USER_AGENT, Accept: "application/json, text/html;q=0.9, */*;q=0.8" },
          signal: AbortSignal.timeout(SEC_TIMEOUT_MS),
          redirect: "follow",
        });
        const bytes = new Uint8Array(await res.arrayBuffer());
        this.log(`fetch  ${res.status} ${url} (${bytes.byteLength} bytes, attempt ${attempt})`);
        if (!RETRY_STATUSES.has(res.status)) return { status: res.status, bytes, contentType: res.headers.get("content-type") };
        problem = `HTTP ${res.status}`;
        waitMs = retryAfterMs(res.headers.get("retry-after"));
      } catch (error) {
        problem = error instanceof Error ? `${error.name}: ${error.message}` : String(error);
        this.log(`fetch  failed ${url} (attempt ${attempt}): ${problem}`);
      }
      if (attempt < MAX_ATTEMPTS) {
        this.stats.retries++;
        await delay(waitMs ?? Math.min(1000 * 2 ** (attempt - 1), MAX_BACKOFF_MS));
      }
    }
    throw new SecHttpError(url, 0, `SEC request failed after ${MAX_ATTEMPTS} attempts (${problem}): ${url}`);
  }

  /** A 200 or 404 response, from the cache or the network depending on the mode. Other statuses throw. */
  async get(url: string): Promise<SecResponse> {
    if (this.mode !== "refresh") {
      const cached = this.fromCache(url);
      if (cached) {
        this.stats.cache++;
        return cached;
      }
      if (this.mode === "replay") throw new SecReplayMissError(url);
    }
    const fetchedAt = new Date().toISOString();
    const { status, bytes, contentType } = await this.fetchNetwork(url);
    if (!CACHEABLE_STATUSES.has(status)) {
      throw new SecHttpError(url, status, `SEC returned HTTP ${status} for ${url}`);
    }
    const sha256 = this.record(url, status, bytes, fetchedAt, contentType);
    return { url, status, body: new TextDecoder().decode(bytes), fetchedAt, sha256, fromCache: false };
  }

  /** Parsed JSON of a 200 response. A 404 (or any other status) throws SecHttpError. */
  async getJson<T = unknown>(url: string): Promise<{ data: T; fetchedAt: string; fromCache: boolean; sha256: string }> {
    const response = await this.get(url);
    if (response.status !== 200) {
      throw new SecHttpError(url, response.status, `SEC returned HTTP ${response.status} for ${url}`);
    }
    try {
      return { data: JSON.parse(response.body) as T, fetchedAt: response.fetchedAt, fromCache: response.fromCache, sha256: response.sha256 };
    } catch {
      throw new SecHttpError(url, response.status, `SEC returned invalid JSON for ${url}`);
    }
  }
}
