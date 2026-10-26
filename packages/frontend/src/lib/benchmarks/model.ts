/**
 * Query helpers over the validated benchmark content. `createBenchmarkModel`
 * takes the content as input so tests can build small datasets; pages use
 * the `benchmarkData` instance from ./index.
 */
import { BENCHMARKS, CATEGORY_ORDER, type BenchmarkCategory, type BenchmarkDefinition } from "./definitions";
import type { BenchmarkContent } from "./content";
import type { BenchmarkMeta, BenchmarkResult, ModelSpec, Offering, SourceId } from "./schema";

/** No new result for this long before the snapshot date reads as stale. */
export const STALE_AFTER_DAYS = 120;
/** A top score within this many points (0..1 scale) of Epoch's ceiling reads as near saturation. */
export const SATURATION_MARGIN = 0.03;

export interface SnapshotInfo {
  source: SourceId;
  retrievedAt: string;
  dataDate: string | null;
  fetchUrls: string[];
}

export interface ResolvedBenchmark {
  definition: BenchmarkDefinition;
  results: BenchmarkResult[];
  meta: BenchmarkMeta | null;
  snapshot: SnapshotInfo | null;
  /** Distinct Tokscale model keys with a result. */
  modelCount: number;
  /** Distinct external models, matched or not. */
  externalModelCount: number;
  /** Newest run, publish or (as a fallback) model release date in the results. */
  newestDate: string | null;
  saturationNote: string | null;
  staleNote: string | null;
}

export interface ModelBenchmarkEntry {
  benchmark: ResolvedBenchmark;
  /** The best result for this model (highest score; newest on ties). */
  best: BenchmarkResult;
  /** Every result for this model, best first. */
  results: BenchmarkResult[];
  /** Rank of `best` among matched models' best results (1 = top). */
  rank: number;
  rankedModels: number;
}

function resultDate(r: BenchmarkResult): string | null {
  return r.runDate ?? r.publishedDate ?? r.modelReleaseDate;
}

/** Higher score first, then newer, then the external id for stability. */
export function compareResults(a: BenchmarkResult, b: BenchmarkResult): number {
  if (b.score !== a.score) return b.score - a.score;
  const da = resultDate(a) ?? "";
  const db = resultDate(b) ?? "";
  if (da !== db) return da < db ? 1 : -1;
  return a.externalId < b.externalId ? -1 : a.externalId > b.externalId ? 1 : 0;
}

/** Best result per model key. Unmatched results are ignored. */
export function bestByModel(results: readonly BenchmarkResult[]): Map<string, BenchmarkResult> {
  const best = new Map<string, BenchmarkResult>();
  for (const result of results) {
    if (!result.modelKey) continue;
    const current = best.get(result.modelKey);
    if (!current || compareResults(result, current) < 0) best.set(result.modelKey, result);
  }
  return best;
}

function daysBetween(a: string, b: string): number {
  return Math.round((Date.parse(`${b.slice(0, 10)}T00:00:00Z`) - Date.parse(`${a.slice(0, 10)}T00:00:00Z`)) / 86_400_000);
}

export function saturationNote(def: BenchmarkDefinition, meta: BenchmarkMeta | null, topScore: number | null): string | null {
  if (def.unit !== "percent" || !meta || meta.ceiling === null || topScore === null) return null;
  if (topScore < meta.ceiling - SATURATION_MARGIN) return null;
  return `Near its ceiling: the top score is ${(topScore * 100).toFixed(1)}%, and Epoch AI estimates the reachable maximum at ${(meta.ceiling * 100).toFixed(0)}%. Differences between top models are small.`;
}

export function staleNote(newestDate: string | null, snapshotDate: string | null): string | null {
  if (!newestDate || !snapshotDate) return null;
  const days = daysBetween(newestDate, snapshotDate);
  if (days <= STALE_AFTER_DAYS) return null;
  return `No new results in the ${days} days before this snapshot.`;
}

export interface BenchmarkModel {
  list(): ResolvedBenchmark[];
  get(slug: string): ResolvedBenchmark | null;
  byCategory(): Array<{ category: BenchmarkCategory; benchmarks: ResolvedBenchmark[] }>;
  /** Best result per model key for one benchmark. */
  best(slug: string): Map<string, BenchmarkResult>;
  /** Every tracked benchmark this model has a result in, in definition order. */
  forModel(key: string): ModelBenchmarkEntry[];
  spec(key: string): ModelSpec | null;
  /** Keys that have a spec or any matched result. */
  modelKeys(): Set<string>;
  provider(id: string): { name: string; url: string | null } | null;
  /** The list price used for estimates: the first offering (the lab's own API when listed). */
  referenceOffering(key: string): Offering | null;
  catalogInfo(): { retrievedAt: string; fetchUrls: string[] };
  snapshots(): SnapshotInfo[];
}

export function createBenchmarkModel(content: BenchmarkContent): BenchmarkModel {
  const resolved = new Map<string, ResolvedBenchmark>();
  const bestCache = new Map<string, Map<string, BenchmarkResult>>();
  for (const definition of BENCHMARKS) {
    const snapshot = definition.source ? content.sources.get(definition.source) ?? null : null;
    const entry = snapshot?.benchmarks.find((b) => b.slug === definition.slug) ?? null;
    const results = entry ? [...entry.results].sort(compareResults) : [];
    const best = bestByModel(results);
    bestCache.set(definition.slug, best);
    let newest: string | null = null;
    for (const r of results) {
      const d = resultDate(r);
      if (d && (newest === null || d > newest)) newest = d;
    }
    const top = results.length > 0 ? results[0].score : null;
    const snapshotDate = snapshot?.retrievedAt.slice(0, 10) ?? null;
    resolved.set(definition.slug, {
      definition,
      results,
      meta: entry?.meta ?? null,
      snapshot: snapshot
        ? { source: snapshot.source, retrievedAt: snapshot.retrievedAt, dataDate: snapshot.dataDate, fetchUrls: snapshot.fetchUrls }
        : null,
      modelCount: best.size,
      externalModelCount: new Set(results.map((r) => r.externalModel)).size,
      newestDate: newest,
      saturationNote: saturationNote(definition, entry?.meta ?? null, top),
      staleNote: definition.status === "active" ? staleNote(newest, snapshotDate) : null,
    });
  }

  const specs = new Map(content.catalog.models.map((m) => [m.modelKey, m] as const));

  const forModel = (key: string): ModelBenchmarkEntry[] => {
    const out: ModelBenchmarkEntry[] = [];
    for (const benchmark of resolved.values()) {
      const best = bestCache.get(benchmark.definition.slug);
      const mine = best?.get(key);
      if (!best || !mine) continue;
      const ranking = [...best.values()].sort(compareResults);
      out.push({
        benchmark,
        best: mine,
        results: benchmark.results.filter((r) => r.modelKey === key),
        rank: ranking.indexOf(mine) + 1,
        rankedModels: ranking.length,
      });
    }
    return out;
  };

  return {
    list: () => [...resolved.values()],
    get: (slug) => resolved.get(slug) ?? null,
    byCategory: () =>
      CATEGORY_ORDER.map((category) => ({
        category,
        benchmarks: [...resolved.values()].filter((b) => b.definition.category === category),
      })).filter((group) => group.benchmarks.length > 0),
    best: (slug) => bestCache.get(slug) ?? new Map(),
    forModel,
    spec: (key) => specs.get(key) ?? null,
    modelKeys: () => {
      const keys = new Set(specs.keys());
      for (const best of bestCache.values()) for (const key of best.keys()) keys.add(key);
      return keys;
    },
    provider: (id) => content.catalog.providers[id] ?? null,
    referenceOffering: (key) => specs.get(key)?.offerings[0] ?? null,
    catalogInfo: () => ({ retrievedAt: content.catalog.retrievedAt, fetchUrls: content.catalog.fetchUrls }),
    snapshots: () =>
      [...content.sources.values()].map((s) => ({ source: s.source, retrievedAt: s.retrievedAt, dataDate: s.dataDate, fetchUrls: s.fetchUrls })),
  };
}
