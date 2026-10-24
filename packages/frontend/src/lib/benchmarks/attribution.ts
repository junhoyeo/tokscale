/**
 * Plain data about benchmarks and sources for client components: column
 * headers, popovers and the "Sources and attribution" footer.
 */
import type { BenchmarkUnit } from "./definitions";
import type { BenchmarkModel, ResolvedBenchmark } from "./model";
import type { SourceId } from "./schema";

export interface BenchmarkInfo {
  slug: string;
  name: string;
  shortName: string;
  unit: BenchmarkUnit;
  unitLabel: string;
  publisher: string;
  collectedBy: string | null;
  url: string;
  measures: string;
  attribution: string;
  licenseName: string;
  licenseUrl: string | null;
  /** When the snapshot was retrieved (ISO), null for link-only benchmarks. */
  retrievedAt: string | null;
}

export function benchmarkInfo(resolved: ResolvedBenchmark): BenchmarkInfo {
  const d = resolved.definition;
  return {
    slug: d.slug,
    name: d.name,
    shortName: d.shortName,
    unit: d.unit,
    unitLabel: d.unitLabel,
    publisher: d.publisher,
    collectedBy: d.collectedBy,
    url: d.url,
    measures: d.measures,
    attribution: d.attribution,
    licenseName: d.license.name,
    licenseUrl: d.license.url,
    retrievedAt: resolved.snapshot?.retrievedAt ?? null,
  };
}

export interface SourceNote {
  id: SourceId | "models.dev" | "tokscale";
  name: string;
  /** What the source contributes on this page. */
  provides: string;
  license: string;
  licenseUrl: string | null;
  url: string;
  /** ISO timestamp of retrieval; null for live Tokscale data. */
  retrievedAt: string | null;
  /** Newest date the source puts on its data. */
  dataDate: string | null;
  credit: string;
}

const SOURCE_TEXT: Record<SourceId, Omit<SourceNote, "retrievedAt" | "dataDate">> = {
  epoch: {
    id: "epoch",
    name: "Epoch AI",
    provides: "Epoch Capabilities Index, Epoch's own runs and the external results it collects",
    license: "CC BY 4.0",
    licenseUrl: "https://creativecommons.org/licenses/by/4.0/",
    url: "https://epoch.ai/benchmarks",
    credit: "Epoch AI, \u2018Capabilities & benchmarking\u2019. Published online at epoch.ai. Retrieved from https://epoch.ai/benchmarks.",
  },
  lmarena: {
    id: "lmarena",
    name: "LMArena",
    provides: "WebDev Arena and Agent Arena leaderboards",
    license: "CC BY 4.0",
    licenseUrl: "https://creativecommons.org/licenses/by/4.0/",
    url: "https://huggingface.co/datasets/lmarena-ai/leaderboard-dataset",
    credit: "LMArena leaderboard dataset (lmarena-ai/leaderboard-dataset), Hugging Face.",
  },
  tbench: {
    id: "tbench",
    name: "Terminal-Bench",
    provides: "Terminal-Bench 4.0 official leaderboard rows (results only, no task content)",
    license: "Terms not stated",
    licenseUrl: null,
    url: "https://www.tbench.ai/leaderboard",
    credit: "Terminal-Bench 4.0 leaderboard, tbench.ai. Every score links back to the leaderboard.",
  },
};

const CATALOG_TEXT = {
  id: "models.dev" as const,
  name: "models.dev",
  provides: "Specs and list prices per provider",
  license: "MIT",
  licenseUrl: "https://github.com/anomalyco/models.dev/blob/dev/LICENSE",
  url: "https://models.dev",
  credit: "models.dev, an open catalog of AI models (github.com/anomalyco/models.dev).",
};

/** Notes for the sources behind `slugs` (all benchmarks when omitted), plus models.dev when asked. */
export function sourceNotes(model: BenchmarkModel, options: { slugs?: readonly string[]; includeCatalog?: boolean } = {}): SourceNote[] {
  const wanted = new Set<SourceId>();
  for (const b of model.list()) {
    if (b.definition.source && (!options.slugs || options.slugs.includes(b.definition.slug))) wanted.add(b.definition.source);
  }
  const notes: SourceNote[] = [];
  for (const snapshot of model.snapshots()) {
    if (!wanted.has(snapshot.source)) continue;
    notes.push({ ...SOURCE_TEXT[snapshot.source], retrievedAt: snapshot.retrievedAt, dataDate: snapshot.dataDate });
  }
  notes.sort((a, b) => (a.id < b.id ? -1 : 1));
  if (options.includeCatalog) notes.push({ ...CATALOG_TEXT, retrievedAt: model.catalogInfo().retrievedAt, dataDate: null });
  return notes;
}
