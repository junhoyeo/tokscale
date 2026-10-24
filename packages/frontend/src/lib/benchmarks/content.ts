/**
 * The validated benchmark dataset. Validation runs once at module load: a
 * schema error or a broken cross-reference throws, so `next build` and the
 * test suite fail before a wrong number can render.
 *
 * Server code only. The snapshots are large; pass client components the
 * rows they show, never this module.
 */
import epochJson from "@/content/benchmarks/epoch.json";
import lmarenaJson from "@/content/benchmarks/lmarena.json";
import modelsDevJson from "@/content/benchmarks/models-dev.json";
import reportJson from "@/content/benchmarks/report.json";
import tbenchJson from "@/content/benchmarks/tbench.json";
import { BENCHMARKS } from "./definitions";
import {
  catalogSnapshotSchema,
  reportSchema,
  sourceSnapshotSchema,
  type CatalogSnapshot,
  type SourceId,
  type SourceSnapshot,
  type SyncReport,
} from "./schema";

export interface BenchmarkContent {
  sources: Map<SourceId, SourceSnapshot>;
  catalog: CatalogSnapshot;
  report: SyncReport;
}

export class BenchmarkContentError extends Error {
  constructor(message: string) {
    super(`[benchmark content] ${message}`);
    this.name = "BenchmarkContentError";
  }
}

function check(condition: boolean, message: string): asserts condition {
  if (!condition) throw new BenchmarkContentError(message);
}

export function validateBenchmarkContent(raw: { sources: unknown[]; catalog: unknown; report: unknown }): BenchmarkContent {
  const sources = new Map<SourceId, SourceSnapshot>();
  for (const file of raw.sources) {
    const snapshot = sourceSnapshotSchema.parse(file);
    check(!sources.has(snapshot.source), `duplicate snapshot for ${snapshot.source}`);
    sources.set(snapshot.source, snapshot);
  }
  const catalog = catalogSnapshotSchema.parse(raw.catalog);
  const report = reportSchema.parse(raw.report);

  const specKeys = new Set<string>();
  for (const model of catalog.models) {
    check(!specKeys.has(model.modelKey), `models.dev snapshot lists ${model.modelKey} twice`);
    specKeys.add(model.modelKey);
    for (const offering of model.offerings) {
      check(Boolean(catalog.providers[offering.provider]), `${model.modelKey}: unknown provider ${offering.provider}`);
    }
  }

  for (const def of BENCHMARKS) {
    if (def.source === null) continue;
    const snapshot = sources.get(def.source);
    check(Boolean(snapshot), `${def.slug}: no ${def.source} snapshot`);
    const entry = snapshot?.benchmarks.find((b) => b.slug === def.slug);
    check(Boolean(entry), `${def.slug}: missing from the ${def.source} snapshot`);
    check(Boolean(entry && entry.results.length > 0), `${def.slug}: no results`);
  }
  for (const snapshot of sources.values()) {
    for (const benchmark of snapshot.benchmarks) {
      check(
        BENCHMARKS.some((def) => def.slug === benchmark.slug && def.source === snapshot.source),
        `${snapshot.source} snapshot has unknown benchmark ${benchmark.slug}`,
      );
      for (const result of benchmark.results) {
        check((result.modelKey === null) === (result.mapping === null), `${benchmark.slug}: ${result.externalId} has a key without a method`);
      }
    }
  }
  return { sources, catalog, report };
}

export const benchmarkContent: BenchmarkContent = validateBenchmarkContent({
  sources: [epochJson, lmarenaJson, tbenchJson],
  catalog: modelsDevJson,
  report: reportJson,
});
