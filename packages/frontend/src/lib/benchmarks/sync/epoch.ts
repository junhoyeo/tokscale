/**
 * Epoch AI benchmarking hub (https://epoch.ai/data/benchmark_data.zip, CC BY 4.0).
 *
 * Reads the unzipped CSVs: the Epoch Capabilities Index, Epoch's own runs
 * (GPQA Diamond, FrontierMath, SimpleQA Verified, SWE-bench Verified) and the
 * external results Epoch collects (CursorBench, DeepSWE, FrontierSWE, HLE).
 */
import { csvDate, csvNumber, parseCsvRecords, roundDerived } from "../csv";
import { agentClientKey, normalizeEffort, splitEpochVersion } from "../conditions";
import { getBenchmarkDefinition } from "../definitions";
import type { BenchmarkMeta, BenchmarkResult, BenchmarkResults, SourceSnapshot } from "../schema";
import { mapExternalModel, type SyncContext } from "./context";

export const EPOCH_ZIP_URL = "https://epoch.ai/data/benchmark_data.zip";

interface ColumnSpec {
  file: string;
  score: string;
  effort?: string;
  agent?: string;
  stderr?: string;
  ciHalfWidth?: string;
  costPerTask?: string;
  tokensPerTask?: string;
  name?: string;
  rank?: string;
  runDate?: string;
  notes?: string;
  sourceColumn?: string;
}

const EPOCH_RUN = (file: string): ColumnSpec => ({
  file,
  score: "Best score (across scorers)",
  stderr: "stderr",
  runDate: "Started at",
});

/** Benchmark slug -> CSV layout. Epoch-run files share one layout. */
export const EPOCH_FILES: Record<string, ColumnSpec> = {
  "gpqa-diamond": EPOCH_RUN("gpqa_diamond.csv"),
  "frontiermath-tiers-1-3": EPOCH_RUN("frontiermath_tiers_1_3_v2.csv"),
  "simpleqa-verified": EPOCH_RUN("simpleqa_verified.csv"),
  "swe-bench-verified": EPOCH_RUN("swe_bench_verified.csv"),
  cursorbench: {
    file: "cursorbench_external.csv",
    score: "Score",
    effort: "Reasoning level",
    costPerTask: "Cost per task",
    tokensPerTask: "Tokens per task",
    name: "Name",
    rank: "Rank",
    sourceColumn: "Source",
  },
  deepswe: {
    file: "deepswe_external.csv",
    score: "Pass@1",
    effort: "Reasoning effort",
    agent: "Harness",
    ciHalfWidth: "95% CI half-width",
    costPerTask: "Mean cost (USD)",
    name: "Name",
    sourceColumn: "Source",
  },
  frontierswe: {
    file: "frontierswe_external.csv",
    score: "Score",
    agent: "Harness",
    costPerTask: "Average cost (USD)",
    name: "Name",
    notes: "Notes",
    sourceColumn: "Source",
  },
  hle: {
    file: "hle_external.csv",
    score: "Accuracy",
    stderr: "Accuracy Standard Error",
    name: "Name",
    notes: "Notes",
  },
};

export const ECI_FILE = "epoch_capabilities_index/eci_scores.csv";
export const MODEL_METADATA_FILE = "model_metadata.csv";
export const BENCHMARK_METADATA_FILE = "benchmark_metadata.csv";

/** Every file the parser reads, relative to the unzipped archive. */
export function epochRequiredFiles(): string[] {
  return [ECI_FILE, MODEL_METADATA_FILE, BENCHMARK_METADATA_FILE, ...Object.values(EPOCH_FILES).map((spec) => spec.file)];
}

function text(value: string | undefined): string | null {
  const trimmed = value?.trim() ?? "";
  return trimmed.length > 0 ? trimmed : null;
}

function httpsUrl(value: string | undefined): string | null {
  const trimmed = value?.trim() ?? "";
  return /^https:\/\/\S+$/.test(trimmed) ? trimmed : null;
}

export function parseBenchmarkMetadata(csv: string): Map<string, BenchmarkMeta> {
  const out = new Map<string, BenchmarkMeta>();
  for (const row of parseCsvRecords(csv)) {
    const name = text(row.benchmark);
    if (!name) continue;
    out.set(name, {
      randomBaseline: csvNumber(row.random_baseline),
      ceiling: csvNumber(row.score_ceiling),
      releaseDate: csvDate(row.release_date),
      supersededBy: text(row.superseded_by),
    });
  }
  return out;
}

/** model_group -> model ids (condition suffix removed), in file order. */
export function parseModelGroups(csv: string): Map<string, string[]> {
  const out = new Map<string, string[]>();
  for (const row of parseCsvRecords(csv)) {
    const group = text(row.model_group);
    const version = text(row.model_version);
    if (!group || !version) continue;
    const base = splitEpochVersion(version).base;
    const list = out.get(group) ?? [];
    if (!list.includes(base)) list.push(base);
    out.set(group, list);
  }
  return out;
}

function emptyResult(): Omit<BenchmarkResult, "externalId" | "externalModel" | "score" | "sourceUrl"> {
  return {
    name: null,
    modelKey: null,
    mapping: null,
    reviewed: false,
    effort: null,
    thinkingBudget: null,
    agent: null,
    agentKey: null,
    ciLow: null,
    ciHigh: null,
    stderr: null,
    votes: null,
    sessions: null,
    rank: null,
    costUsd: null,
    costBasis: null,
    tokens: null,
    modelReleaseDate: null,
    runDate: null,
    publishedDate: null,
    notes: null,
  };
}

export function parseEciScores(csv: string, groups: Map<string, string[]>, ctx: SyncContext): BenchmarkResult[] {
  const def = getBenchmarkDefinition("epoch-capabilities-index");
  if (!def) throw new Error("missing ECI definition");
  const results: BenchmarkResult[] = [];
  for (const row of parseCsvRecords(csv)) {
    const model = text(row.Model);
    const score = csvNumber(row.eci);
    if (!model || score === null) continue;
    const display = text(row["Display name"]) ?? model;
    const date = csvDate(row.date);
    const resolution = mapExternalModel(
      ctx,
      "epoch",
      { externalId: model, ids: groups.get(model) ?? [], names: [display, model], org: text(row.Organization) },
      { benchmark: def.slug, name: display, date },
    );
    results.push({
      ...emptyResult(),
      externalId: model,
      externalModel: model,
      name: display,
      modelKey: resolution.key,
      mapping: resolution.method,
      reviewed: resolution.reviewed,
      score,
      ciLow: csvNumber(row.eci_ci_low),
      ciHigh: csvNumber(row.eci_ci_high),
      modelReleaseDate: date,
      sourceUrl: def.url,
    });
  }
  return results;
}

/** model version (as written) -> model_group, for name candidates. */
export function parseVersionGroups(csv: string): Map<string, string> {
  const out = new Map<string, string>();
  for (const row of parseCsvRecords(csv)) {
    const group = text(row.model_group);
    const version = text(row.model_version);
    if (group && version) out.set(version, group);
  }
  return out;
}

export function parseEpochBenchmark(
  slug: string,
  csv: string,
  ctx: SyncContext,
  versionGroups: ReadonlyMap<string, string> = new Map(),
): BenchmarkResult[] {
  const def = getBenchmarkDefinition(slug);
  const spec = EPOCH_FILES[slug];
  if (!def || !spec) throw new Error(`no Epoch layout for ${slug}`);
  const results: BenchmarkResult[] = [];
  for (const row of parseCsvRecords(csv)) {
    const version = text(row["Model version"]);
    const score = csvNumber(row[spec.score]);
    if (!version || score === null) continue;
    const split = splitEpochVersion(version);
    const name = spec.name ? text(row[spec.name]) : null;
    const releaseDate = csvDate(row["Release date"]);
    const runDate = spec.runDate ? csvDate(row[spec.runDate]) : null;
    const resolution = mapExternalModel(
      ctx,
      "epoch",
      {
        externalId: split.base,
        ids: [split.base],
        names: [name, versionGroups.get(version) ?? null].filter((n): n is string => Boolean(n)),
        org: text(row.Organization),
      },
      { benchmark: slug, name, date: runDate ?? releaseDate },
    );
    const effort = (spec.effort ? normalizeEffort(row[spec.effort]) : null) ?? split.effort;
    const agent = spec.agent ? text(row[spec.agent]) : null;
    const halfWidth = spec.ciHalfWidth ? csvNumber(row[spec.ciHalfWidth]) : null;
    const rank = spec.rank ? csvNumber(row[spec.rank]) : null;
    results.push({
      ...emptyResult(),
      externalId: version,
      externalModel: split.base,
      name,
      modelKey: resolution.key,
      mapping: resolution.method,
      reviewed: resolution.reviewed,
      effort,
      thinkingBudget: split.thinkingBudget,
      agent,
      agentKey: agentClientKey(agent),
      score,
      ciLow: halfWidth !== null ? roundDerived(score - halfWidth) : null,
      ciHigh: halfWidth !== null ? roundDerived(score + halfWidth) : null,
      stderr: spec.stderr ? csvNumber(row[spec.stderr]) : null,
      rank: rank !== null && Number.isInteger(rank) && rank > 0 ? rank : null,
      costUsd: spec.costPerTask ? csvNumber(row[spec.costPerTask]) : null,
      costBasis: spec.costPerTask && csvNumber(row[spec.costPerTask]) !== null ? "per task" : null,
      tokens: spec.tokensPerTask ? csvNumber(row[spec.tokensPerTask]) : null,
      modelReleaseDate: releaseDate,
      runDate,
      sourceUrl: (spec.sourceColumn ? httpsUrl(row[spec.sourceColumn]) : null) ?? def.url,
      notes: spec.notes ? text(row[spec.notes]) : null,
    });
  }
  return results;
}

function newestDate(results: BenchmarkResult[]): string | null {
  let newest: string | null = null;
  for (const r of results) {
    for (const date of [r.runDate, r.publishedDate, r.modelReleaseDate]) {
      if (date && (newest === null || date > newest)) newest = date;
    }
  }
  return newest;
}

/** Build the Epoch snapshot from the unzipped files (path -> CSV text). */
export function buildEpochSnapshot(files: Record<string, string>, ctx: SyncContext, retrievedAt: string): SourceSnapshot {
  const read = (name: string) => {
    const content = files[name];
    if (content === undefined) throw new Error(`Epoch archive is missing ${name}`);
    return content;
  };
  const meta = parseBenchmarkMetadata(read(BENCHMARK_METADATA_FILE));
  const groups = parseModelGroups(read(MODEL_METADATA_FILE));
  const versionGroups = parseVersionGroups(read(MODEL_METADATA_FILE));
  const benchmarks: BenchmarkResults[] = [
    {
      slug: "epoch-capabilities-index",
      sourceName: "Epoch Capabilities Index",
      meta: null,
      results: parseEciScores(read(ECI_FILE), groups, ctx),
    },
  ];
  for (const [slug, spec] of Object.entries(EPOCH_FILES)) {
    const def = getBenchmarkDefinition(slug);
    if (!def?.sourceName) throw new Error(`definition for ${slug} has no source name`);
    benchmarks.push({
      slug,
      sourceName: def.sourceName,
      meta: meta.get(def.sourceName) ?? null,
      results: parseEpochBenchmark(slug, read(spec.file), ctx, versionGroups),
    });
  }
  return {
    source: "epoch",
    retrievedAt,
    fetchUrls: [EPOCH_ZIP_URL],
    dataDate: newestDate(benchmarks.flatMap((b) => b.results)),
    benchmarks,
  };
}
