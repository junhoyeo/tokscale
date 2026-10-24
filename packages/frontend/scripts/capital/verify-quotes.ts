/**
 * Check that every quote in the Capital ledger content is copied from the source it cites.
 *
 *   bun scripts/capital/verify-quotes.ts              # use the cache, fetch only what is missing
 *   bun scripts/capital/verify-quotes.ts --offline    # cache only; a page that is not cached fails
 *   bun scripts/capital/verify-quotes.ts --refresh    # fetch every cited page again
 *   bun scripts/capital/verify-quotes.ts events/anthropic-series-g-2026.json   # only these files
 *   bun scripts/capital/verify-quotes.ts --content=/tmp/capital-copy             # another content tree
 *
 * Reads src/content/capital/orgs*.json, events/*.json and metrics/statements*.json. For each source,
 * the document is fetched through scripts/capital/lib/source-cache.ts (SEC at most 2 requests per
 * second with the declared User-Agent; every raw response is cached under .stealth-cache/capital/).
 * A quote passes when, after normalization (whitespace collapsed, curly quotes straightened, spaces
 * before punctuation removed), it is a substring of the page's visible text. If not, the raw document
 * with whitespace collapsed is tried, so XML filings can be quoted with their element tags
 * ("<totalAmountSold>3108543889</totalAmountSold>"). As a last step the visible text is compared with
 * all whitespace removed, because the rule is that only whitespace may differ. Prints one line per
 * quote and a summary. Exits 1 when any quote fails.
 */
import { readdirSync, readFileSync } from "node:fs";
import { isAbsolute, join, relative } from "node:path";
import {
  CacheMissError,
  decodeEntities,
  fetchSource,
  normalizeForMatch,
  visibleText,
  type CacheMode,
  type SourceResponse,
} from "./lib/source-cache";

const DEFAULT_CONTENT = join(import.meta.dir, "..", "..", "src", "content", "capital");
/** `--content=<dir>` checks another copy of the content tree (used to test this script). */
const CONTENT = process.argv.find((arg) => arg.startsWith("--content="))?.slice("--content=".length) || DEFAULT_CONTENT;
const CONCURRENCY = 4;

interface SourceDef {
  id: string;
  url: string;
  type?: string;
}

interface QuoteRef {
  file: string;
  where: string;
  sourceId: string;
  quote: string;
  /** Amounts and quantities should quote the number they support. */
  numeric: boolean;
}

type Json = Record<string, unknown>;
const asArray = (value: unknown): Json[] => (Array.isArray(value) ? (value as Json[]) : []);
const str = (value: unknown): string | undefined => (typeof value === "string" ? value : undefined);

function contentFiles(): string[] {
  const list = (dir: string, pattern: RegExp, prefix: string) =>
    readdirSync(dir)
      .filter((name) => pattern.test(name))
      .sort()
      .map((name) => (prefix ? `${prefix}/${name}` : name));
  return [
    ...list(CONTENT, /^orgs(?:-[a-z0-9-]+)?\.json$/, ""),
    ...list(join(CONTENT, "events"), /\.json$/, "events"),
    ...list(join(CONTENT, "metrics"), /^statements(?:-[a-z0-9-]+)?\.json$/, "metrics"),
  ];
}

function collect(files: string[]): { sources: Map<string, SourceDef>; quotes: QuoteRef[]; problems: string[] } {
  const sources = new Map<string, SourceDef>();
  const quotes: QuoteRef[] = [];
  const problems: string[] = [];
  for (const file of files) {
    let data: Json;
    try {
      data = JSON.parse(readFileSync(join(CONTENT, file), "utf8")) as Json;
    } catch (error) {
      problems.push(`${file}: cannot read JSON (${error instanceof Error ? error.message : String(error)})`);
      continue;
    }
    for (const source of asArray(data.sources)) {
      const id = str(source.id);
      const url = str(source.url);
      if (!id || !url) continue;
      const existing = sources.get(id);
      if (existing && existing.url !== url) problems.push(`${file}: source ${id} has a different URL than in another file`);
      if (!existing) sources.set(id, { id, url, type: str(source.type) });
    }
    const add = (where: string, item: Json, numeric: boolean) => {
      const sourceId = str(item.sourceId);
      const quote = str(item.quote);
      if (sourceId && quote) quotes.push({ file, where, sourceId, quote, numeric });
    };
    for (const org of asArray(data.orgs)) {
      const parent = org.parent as Json | undefined;
      if (parent && typeof parent === "object") add(`org ${str(org.id)} parent`, parent, false);
    }
    const event = data.event as Json | undefined;
    if (event && typeof event === "object") {
      for (const amount of asArray(event.amounts)) add(`amount ${str(amount.id)}`, amount, amount.value !== null);
      for (const quantity of asArray(event.quantities)) add(`quantity ${str(quantity.id)}`, quantity, true);
      for (const fact of asArray(event.facts)) add(`fact "${str(fact.label)}"`, fact, false);
    }
    for (const statement of asArray(data.statements)) add(`statement ${str(statement.id)}`, statement, true);
  }
  return { sources, quotes, problems };
}

interface Checked {
  response?: SourceResponse;
  error?: string;
  /** Normalized visible text. */
  text?: string;
  /** Normalized raw document, as fetched and with entities decoded. */
  raw?: string;
  rawDecoded?: string;
  /** Visible text with every whitespace character removed (the rule is "only whitespace may differ"). */
  textNoSpace?: string;
}

const squash = (text: string) => text.replace(/\s+/g, "");

async function load(source: SourceDef, mode: CacheMode): Promise<Checked> {
  try {
    const response = await fetchSource(source.url, { mode, log: (line) => console.error(`  ${line}`) });
    if (response.status !== 200) return { response, error: `HTTP ${response.status}` };
    if (/application\/pdf/i.test(response.contentType ?? "") || response.body.startsWith("%PDF")) {
      return { response, error: "PDF text extraction is not supported" };
    }
    const text = normalizeForMatch(visibleText(response.body));
    return {
      response,
      text,
      raw: normalizeForMatch(response.body),
      rawDecoded: normalizeForMatch(decodeEntities(response.body)),
      textNoSpace: squash(text),
    };
  } catch (error) {
    if (error instanceof CacheMissError) return { error: "not cached (offline)" };
    return { error: error instanceof Error ? error.message : String(error) };
  }
}

/** Length of the longest prefix of `needle` found in `haystack` (for FAIL hints). */
function longestPrefix(haystack: string, needle: string): number {
  let lo = 0;
  let hi = needle.length;
  while (lo < hi) {
    const mid = Math.ceil((lo + hi) / 2);
    if (haystack.includes(needle.slice(0, mid))) lo = mid;
    else hi = mid - 1;
  }
  return lo;
}

const short = (text: string, max = 70) => (text.length > max ? `${text.slice(0, max - 1)}…` : text);

async function main() {
  const args = process.argv.slice(2);
  const mode: CacheMode = args.includes("--offline") ? "offline" : args.includes("--refresh") ? "refresh" : "cache";
  const only = args
    .filter((arg) => !arg.startsWith("--"))
    .map((arg) => (isAbsolute(arg) ? relative(CONTENT, arg) : arg.replace(/^src\/content\/capital\//, "")));
  const all = contentFiles();
  const files = only.length ? all.filter((file) => only.includes(file)) : all;
  if (only.length && files.length !== only.length) {
    console.error(`unknown content file(s): ${only.filter((file) => !all.includes(file)).join(", ")}`);
    process.exit(2);
  }
  // Sources may be defined in any file, so read them all; check quotes only in the selected files.
  const { sources, problems } = collect(all);
  const { quotes } = collect(files);
  console.log(`verify-quotes: ${files.length} files, ${quotes.length} quotes, mode ${mode}`);
  for (const problem of problems) console.log(`WARN  ${problem}`);

  const needed = [...new Set(quotes.map((q) => q.sourceId))].sort();
  const checked = new Map<string, Checked>();
  const queue = needed.filter((id) => sources.has(id));
  const workers = Array.from({ length: Math.min(CONCURRENCY, queue.length) }, async () => {
    for (let id = queue.shift(); id !== undefined; id = queue.shift()) checked.set(id, await load(sources.get(id)!, mode));
  });
  await Promise.all(workers);

  let pass = 0;
  let passRaw = 0;
  let passSpace = 0;
  let fail = 0;
  let notes = 0;
  for (const q of [...quotes].sort((a, b) => (a.file === b.file ? 0 : a.file < b.file ? -1 : 1))) {
    const head = `${q.file}  ${q.where}  ${q.sourceId}`;
    const source = sources.get(q.sourceId);
    const result = checked.get(q.sourceId);
    if (!source || !result) {
      fail++;
      console.log(`FAIL  ${head}: source is not defined in any content file`);
      continue;
    }
    if (result.error || result.text === undefined) {
      fail++;
      console.log(`FAIL  ${head}: ${result.error ?? "no text"} (${source.url})`);
      continue;
    }
    const needle = normalizeForMatch(q.quote);
    const note = q.numeric && !/\d/.test(needle) ? "  [note: the quote has no digit]" : "";
    if (note) notes++;
    if (result.text.includes(needle)) {
      pass++;
      console.log(`PASS  ${head}${note}`);
    } else if (result.raw!.includes(needle) || result.rawDecoded!.includes(needle)) {
      pass++;
      passRaw++;
      console.log(`PASS  ${head}  (raw document)${note}`);
    } else if (result.textNoSpace!.includes(squash(needle))) {
      pass++;
      passSpace++;
      console.log(`PASS  ${head}  (only whitespace differs)${note}`);
    } else {
      fail++;
      const n = longestPrefix(result.text, needle);
      console.log(`FAIL  ${head}: not in the page text. Matched the first ${n} of ${needle.length} characters; differs at "${short(needle.slice(n), 60)}"`);
    }
  }

  const origins = { network: 0, cache: 0, "sec-cache": 0 } as Record<string, number>;
  let failedSources = 0;
  for (const result of checked.values()) {
    if (result.response) origins[result.response.origin]++;
    if (result.error) failedSources++;
  }
  const undefinedSources = needed.filter((id) => !sources.has(id)).length;
  console.log(
    `\nSummary: ${quotes.length} quotes, ${pass} pass (${passRaw} in the raw document, ${passSpace} with whitespace differences), ${fail} fail` +
      (notes ? `, ${notes} numeric quotes without a digit` : "") +
      `. Sources: ${needed.length} cited, ${origins.network} fetched, ${origins.cache + origins["sec-cache"]} from cache` +
      `, ${failedSources + undefinedSources} unusable.`
  );
  process.exit(fail > 0 ? 1 : 0);
}

await main();
