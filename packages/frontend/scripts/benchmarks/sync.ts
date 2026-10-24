/**
 * Sync benchmark, spec and price snapshots for the Models dashboard.
 *
 * Fetches models.dev, the Epoch AI benchmark archive, the LMArena boards
 * (Hugging Face datasets-server API) and the Terminal-Bench 4.0 leaderboard;
 * maps external model ids to Tokscale model keys; validates with zod; and
 * writes src/content/benchmarks/*.json plus a mapping report. Raw downloads
 * are cached under .stealth-cache/benchmarks (gitignored, never committed).
 *
 * Usage (from packages/frontend):
 *   bun scripts/benchmarks/sync.ts             # fetch everything
 *   bun scripts/benchmarks/sync.ts --offline   # reuse cached downloads only
 *
 * With DATABASE_URL pointing at a local database, usage keys join the set of
 * keys a result may map to, and coverage by tokens and users is printed:
 *   DATABASE_URL=postgresql://tokscale:tokscale@127.0.0.1:5480/tokscale DATABASE_SSL=false \
 *     bun scripts/benchmarks/sync.ts
 *
 * If one source fails, its previous snapshot is kept and the report says so.
 */
import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import path from "node:path";
import { isIdentifiedModelKey } from "@/lib/explore/modelKey";
import type { ModelUsageRow } from "@/lib/explore/types";
import { aliasRuleId, type AliasRule } from "@/lib/benchmarks/mapping";
import {
  aliasFileSchema,
  catalogSnapshotSchema,
  reportSchema,
  sourceSnapshotSchema,
  type CatalogSnapshot,
  type SourceId,
  type SourceSnapshot,
} from "@/lib/benchmarks/schema";
import { aliasRulesForWrite, createSyncContext, type SyncContext } from "@/lib/benchmarks/sync/context";
import { buildEpochSnapshot, epochRequiredFiles, EPOCH_ZIP_URL } from "@/lib/benchmarks/sync/epoch";
import {
  buildArenaSnapshot,
  lmarenaFilterUrl,
  LMARENA_BOARDS,
  LMARENA_PAGE_SIZE,
  parseArenaRows,
  type ArenaBoardSlug,
  type ArenaPage,
  type ArenaRow,
} from "@/lib/benchmarks/sync/lmarena";
import {
  buildCatalogSnapshot,
  catalogKnownKeys,
  firstPartyPricedKeys,
  MODELS_DEV_URL,
  type ModelsDevApi,
} from "@/lib/benchmarks/sync/modelsdev";
import { buildReport, formatSnapshotJson, type SourceStatus } from "@/lib/benchmarks/sync/output";
import { buildTbenchSnapshot, extractTbenchLeaderboard, TBENCH_URL } from "@/lib/benchmarks/sync/tbench";

const USER_AGENT = "Tokscale research i@junho.io";
const MIN_REQUEST_GAP_MS = 250; // at most 4 requests per second
const FETCH_TIMEOUT_MS = 60_000;

const ROOT = path.resolve(import.meta.dir, "../..");
const CACHE_DIR = path.join(ROOT, ".stealth-cache", "benchmarks");
const OUT_DIR = path.join(ROOT, "src", "content", "benchmarks");
const OFFLINE = process.argv.includes("--offline");

let lastRequestAt = 0;

const RETRY_DELAYS_MS = [2_000, 5_000];

function delay(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

/** GET with a declared User-Agent, a request gap, a timeout, and two retries on 5xx or network errors. */
async function politeFetch(url: string): Promise<Response> {
  for (let attempt = 0; ; attempt++) {
    const wait = lastRequestAt + MIN_REQUEST_GAP_MS - Date.now();
    if (wait > 0) await delay(wait);
    lastRequestAt = Date.now();
    try {
      const response = await fetch(url, {
        headers: { "User-Agent": USER_AGENT, Accept: "*/*" },
        signal: AbortSignal.timeout(FETCH_TIMEOUT_MS),
        redirect: "follow",
      });
      if (response.ok) return response;
      if (response.status < 500 || attempt >= RETRY_DELAYS_MS.length) throw new Error(`${url}: HTTP ${response.status}`);
    } catch (error) {
      if (attempt >= RETRY_DELAYS_MS.length || (error instanceof Error && /HTTP 4\d\d/.test(error.message))) throw error;
    }
    await delay(RETRY_DELAYS_MS[attempt]);
  }
}

/** Download `url` into the cache (or reuse the cached copy with --offline). */
async function cached(url: string, file: string, kind: "text" | "binary"): Promise<string> {
  const target = path.join(CACHE_DIR, file);
  if (!OFFLINE) {
    const response = await politeFetch(url);
    const body = kind === "text" ? await response.text() : new Uint8Array(await response.arrayBuffer());
    writeFileSync(target, body);
    writeFileSync(`${target}.meta.json`, JSON.stringify({ url, finalUrl: response.url, fetchedAt: new Date().toISOString() }, null, 2));
  } else if (!existsSync(target)) {
    throw new Error(`--offline: no cached copy of ${url}`);
  }
  return target;
}

function readJson<T>(file: string): T | null {
  return existsSync(file) ? (JSON.parse(readFileSync(file, "utf8")) as T) : null;
}

function writeOut(name: string, value: unknown): void {
  writeFileSync(path.join(OUT_DIR, name), formatSnapshotJson(value));
}

function cachedAt(file: string): string {
  const meta = readJson<{ fetchedAt?: string }>(`${path.join(CACHE_DIR, file)}.meta.json`);
  return meta?.fetchedAt ?? new Date().toISOString();
}

// ---------------------------------------------------------------- sources

async function loadModelsDev(): Promise<{ api: ModelsDevApi; retrievedAt: string }> {
  const file = await cached(MODELS_DEV_URL, "models-dev.json", "text");
  return { api: JSON.parse(readFileSync(file, "utf8")) as ModelsDevApi, retrievedAt: cachedAt("models-dev.json") };
}

async function syncEpoch(ctx: SyncContext): Promise<SourceSnapshot> {
  const zip = await cached(EPOCH_ZIP_URL, "epoch-benchmark-data.zip", "binary");
  const dir = path.join(CACHE_DIR, "epoch");
  mkdirSync(dir, { recursive: true });
  const unzip = spawnSync("unzip", ["-o", "-q", zip, "-d", dir], { timeout: 60_000, encoding: "utf8" });
  if (unzip.status !== 0) throw new Error(`unzip failed: ${unzip.stderr || unzip.error?.message}`);
  const readme = readFileSync(path.join(dir, "README.md"), "utf8");
  if (!/creativecommons\.org\/licenses\/by\/4\.0/.test(readme)) throw new Error("Epoch README no longer states CC BY 4.0");
  const files: Record<string, string> = {};
  for (const name of epochRequiredFiles()) files[name] = readFileSync(path.join(dir, name), "utf8");
  return buildEpochSnapshot(files, ctx, cachedAt("epoch-benchmark-data.zip"));
}

async function fetchArenaBoard(config: string): Promise<{ rows: ArenaRow[]; urls: string[] }> {
  const rows: ArenaRow[] = [];
  const urls: string[] = [];
  for (let offset = 0, page = 0; page < 20; offset += LMARENA_PAGE_SIZE, page++) {
    const url = lmarenaFilterUrl(config, offset);
    const file = await cached(url, `lmarena-${config}-${offset}.json`, "text");
    const body = JSON.parse(readFileSync(file, "utf8")) as ArenaPage;
    urls.push(url);
    rows.push(...body.rows.map((r) => r.row));
    if (offset + LMARENA_PAGE_SIZE >= body.num_rows_total) break;
  }
  return { rows, urls };
}

async function syncArena(ctx: SyncContext): Promise<SourceSnapshot> {
  const boards = {} as Record<ArenaBoardSlug, ArenaRow[]>;
  const fetchUrls: string[] = [];
  let retrievedAt = new Date().toISOString();
  for (const slug of Object.keys(LMARENA_BOARDS) as ArenaBoardSlug[]) {
    const { rows, urls } = await fetchArenaBoard(LMARENA_BOARDS[slug]);
    boards[slug] = rows;
    fetchUrls.push(urls[0]);
    retrievedAt = cachedAt(`lmarena-${LMARENA_BOARDS[slug]}-0.json`);
  }
  return buildArenaSnapshot(boards, ctx, retrievedAt, fetchUrls);
}

async function syncTbench(ctx: SyncContext): Promise<SourceSnapshot> {
  const file = await cached(TBENCH_URL, "tbench-leaderboard.html", "text");
  const board = extractTbenchLeaderboard(readFileSync(file, "utf8"));
  return buildTbenchSnapshot(board, ctx, cachedAt("tbench-leaderboard.html"));
}

// ---------------------------------------------------------------- usage (local DB only)

interface UsageData {
  keys: Set<string>;
  rollups: Record<"7d" | "30d" | "all", ModelUsageRow[]>;
}

async function loadUsage(): Promise<UsageData | null> {
  const url = process.env.DATABASE_URL;
  if (!url) return null;
  const host = new URL(url).hostname;
  if (host !== "127.0.0.1" && host !== "localhost") throw new Error("sync.ts only reads a local DATABASE_URL");
  const { computeCatalog, computePeriodRollup } = await import("@/lib/explore/db");
  const timeout = new Promise<never>((_, reject) => setTimeout(() => reject(new Error("usage queries timed out")), 180_000));
  const work = (async () => {
    const catalog = await computeCatalog();
    const keys = new Set(Object.keys(catalog.aliases).filter(isIdentifiedModelKey));
    const rollups = {
      "7d": (await computePeriodRollup("7d", catalog)).models,
      "30d": (await computePeriodRollup("30d", catalog)).models,
      all: (await computePeriodRollup("all", catalog)).models,
    };
    return { keys, rollups };
  })();
  return Promise.race([work, timeout]);
}

function pct(part: number, total: number): string {
  return total > 0 ? `${((part / total) * 100).toFixed(1)}%` : "n/a";
}

function coverageLines(usage: UsageData, snapshots: SourceSnapshot[], catalog: CatalogSnapshot | null): string[] {
  const lines: string[] = [];
  for (const period of ["30d", "7d", "all"] as const) {
    const rows = usage.rollups[period];
    const tokens = rows.reduce((s, r) => s + r.tokens, 0);
    const pairs = rows.reduce((s, r) => s + r.activeUsers, 0);
    const line = (label: string, keys: Set<string>) => {
      const covered = rows.filter((r) => keys.has(r.key));
      const t = covered.reduce((s, r) => s + r.tokens, 0);
      const p = covered.reduce((s, r) => s + r.activeUsers, 0);
      const top = rows.filter((r) => r.activeUsers >= 5);
      const topCovered = top.filter((r) => keys.has(r.key)).length;
      lines.push(
        `  ${label.padEnd(26)} models ${String(covered.length).padStart(4)}/${rows.length}  tokens ${pct(t, tokens).padStart(6)}  user-model pairs ${pct(p, pairs).padStart(6)}  5+ user models ${topCovered}/${top.length}`,
      );
    };
    lines.push(`Coverage against the local database, ${period} (${rows.length} models):`);
    line("models.dev specs", new Set(catalog?.models.map((m) => m.modelKey) ?? []));
    line("models.dev first-party price", new Set(catalog?.models.filter((m) => m.labProvider).map((m) => m.modelKey) ?? []));
    for (const snapshot of snapshots) {
      for (const benchmark of snapshot.benchmarks) {
        line(benchmark.slug, new Set(benchmark.results.flatMap((r) => (r.modelKey ? [r.modelKey] : []))));
      }
    }
    if (period === "30d") {
      const anyBench = new Set(snapshots.flatMap((s) => s.benchmarks.flatMap((b) => b.results.flatMap((r) => (r.modelKey ? [r.modelKey] : [])))));
      const missing = rows.filter((r) => !anyBench.has(r.key)).sort((a, b) => b.tokens - a.tokens).slice(0, 15);
      lines.push(`  Top 30d models with no benchmark result: ${missing.map((r) => `${r.key} (${r.activeUsers} users)`).join(", ")}`);
    }
  }
  return lines;
}

// ---------------------------------------------------------------- main

async function main(): Promise<void> {
  mkdirSync(CACHE_DIR, { recursive: true });
  mkdirSync(OUT_DIR, { recursive: true });
  const generatedAt = new Date().toISOString();

  const previousAliases = readJson<unknown>(path.join(OUT_DIR, "aliases.json"));
  const rules: AliasRule[] = previousAliases ? aliasFileSchema.parse(previousAliases).aliases : [];
  const previousCatalog = readJson<unknown>(path.join(OUT_DIR, "models-dev.json"));

  const statuses: SourceStatus[] = [];
  let api: ModelsDevApi | null = null;
  let catalogRetrievedAt = generatedAt;
  try {
    const loaded = await loadModelsDev();
    api = loaded.api;
    catalogRetrievedAt = loaded.retrievedAt;
    statuses.push({ source: "models.dev", ok: true, error: null });
  } catch (error) {
    statuses.push({ source: "models.dev", ok: false, error: String(error) });
    console.error(`[sync] models.dev failed, keeping the previous snapshot: ${error}`);
  }

  const usage = await loadUsage();
  const catalogKeys = api
    ? catalogKnownKeys(api)
    : new Set(previousCatalog ? catalogSnapshotSchema.parse(previousCatalog).models.map((m) => m.modelKey) : []);
  const knownKeys = new Set([...catalogKeys, ...(usage?.keys ?? [])]);
  const ctx = createSyncContext(knownKeys, rules);

  const sources: Array<{ id: SourceId; file: string; run: (c: SyncContext) => Promise<SourceSnapshot> }> = [
    { id: "epoch", file: "epoch.json", run: syncEpoch },
    { id: "lmarena", file: "lmarena.json", run: syncArena },
    { id: "tbench", file: "tbench.json", run: syncTbench },
  ];
  const snapshots: SourceSnapshot[] = [];
  for (const source of sources) {
    try {
      const snapshot = sourceSnapshotSchema.parse(await source.run(ctx));
      snapshots.push(snapshot);
      writeOut(source.file, snapshot);
      statuses.push({ source: source.id, ok: true, error: null });
    } catch (error) {
      statuses.push({ source: source.id, ok: false, error: String(error) });
      console.error(`[sync] ${source.id} failed, keeping the previous snapshot: ${error}`);
      const previous = readJson<unknown>(path.join(OUT_DIR, source.file));
      if (previous) snapshots.push(sourceSnapshotSchema.parse(previous));
      // Keep that source's alias rules as they were.
      for (const rule of rules) {
        if (rule.source === source.id) ctx.decisions.set(aliasRuleId(rule.source, rule.externalId), rule);
      }
    }
  }

  let catalog: CatalogSnapshot | null = null;
  if (api) {
    const wanted = new Set<string>([
      ...snapshots.flatMap((s) => s.benchmarks.flatMap((b) => b.results.flatMap((r) => (r.modelKey ? [r.modelKey] : [])))),
      ...(usage?.keys ?? []),
      ...firstPartyPricedKeys(api),
    ]);
    catalog = catalogSnapshotSchema.parse(buildCatalogSnapshot(api, wanted, catalogRetrievedAt));
    writeOut("models-dev.json", catalog);
  } else if (previousCatalog) {
    catalog = catalogSnapshotSchema.parse(previousCatalog);
  }

  const aliasRules = aliasRulesForWrite(ctx);
  writeOut("aliases.json", aliasFileSchema.parse({ aliases: aliasRules }));
  const report = reportSchema.parse(buildReport(ctx, snapshots, catalog, statuses, aliasRules, generatedAt));
  writeOut("report.json", report);

  // Text arena: fetched for coverage only, not published (not in the first benchmark set).
  try {
    const text = await fetchArenaBoard("text");
    const probe = createSyncContext(knownKeys, rules);
    const mapped = parseArenaRows("webdev-arena", text.rows, probe).filter((r) => r.modelKey).length;
    console.log(`Text arena (coverage only): ${mapped}/${text.rows.length} overall rows map to a model key.`);
  } catch (error) {
    console.error(`[sync] text arena probe failed: ${error}`);
  }

  console.log(`\nSnapshots written to ${path.relative(ROOT, OUT_DIR)} (${readdirSync(OUT_DIR).join(", ")})`);
  for (const s of report.sources) {
    console.log(
      `  ${s.source.padEnd(10)} ${s.ok ? "ok  " : "KEPT"} results ${s.mappedResults}/${s.results} mapped, models ${s.mappedModels}/${s.externalModels}${s.error ? `  (${s.error})` : ""}`,
    );
  }
  console.log(`  alias methods: ${JSON.stringify(report.methods)}; unmapped external models: ${report.unmapped.length}`);
  if (usage) {
    const lines = coverageLines(usage, snapshots, catalog);
    console.log(`\n${lines.join("\n")}`);
    writeFileSync(path.join(CACHE_DIR, "coverage.txt"), `${generatedAt}\n${lines.join("\n")}\n`);
  } else {
    console.log("\nDATABASE_URL not set: usage keys and coverage skipped.");
  }
  process.exit(statuses.every((s) => s.ok) ? 0 : 2);
}

await main();
