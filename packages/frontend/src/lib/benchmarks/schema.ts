import { z } from "zod";
import { EFFORTS } from "./conditions";

/**
 * Zod schemas for the benchmark snapshots in src/content/benchmarks.
 *
 * scripts/benchmarks/sync.ts writes these files; ./content.ts validates them
 * once at module load, so a malformed snapshot fails the build and the tests
 * instead of rendering a wrong number.
 */

const isoDate = z.string().regex(/^\d{4}-\d{2}-\d{2}$/, "expected YYYY-MM-DD");
const isoDateTime = z.string().datetime();
const httpsUrl = z.string().url().startsWith("https://");
const modelKey = z.string().regex(/^[a-z0-9][a-z0-9._-]{0,95}$/, "expected a canonical model key");
const nonNegative = z.number().finite().nonnegative();

export const SOURCE_IDS = ["epoch", "lmarena", "tbench"] as const;
export const sourceIdSchema = z.enum(SOURCE_IDS);
export type SourceId = z.infer<typeof sourceIdSchema>;

export const mappingMethodSchema = z.enum(["exact", "normalized", "manual"]);

export const resultSchema = z.object({
  /** Row id as written by the source (model version, board name). */
  externalId: z.string().min(1),
  /** Model part of the id without conditions; the alias map is keyed on it. */
  externalModel: z.string().min(1),
  /** Display name in the source, when it gives one. */
  name: z.string().nullable(),
  modelKey: modelKey.nullable(),
  mapping: mappingMethodSchema.nullable(),
  reviewed: z.boolean(),
  effort: z.enum(EFFORTS).nullable(),
  thinkingBudget: z.string().nullable(),
  /** Agent or harness that ran the model, as named by the source. */
  agent: z.string().nullable(),
  /** Tokscale client key of that agent, when Tokscale tracks it. */
  agentKey: z.string().nullable(),
  /** In the benchmark's unit: a 0..1 fraction for percentages, raw otherwise. */
  score: z.number().finite(),
  ciLow: z.number().finite().nullable(),
  ciHigh: z.number().finite().nullable(),
  stderr: nonNegative.nullable(),
  votes: nonNegative.nullable(),
  sessions: nonNegative.nullable(),
  /** Rank as published by the source, when it publishes one. */
  rank: z.number().int().positive().nullable(),
  costUsd: nonNegative.nullable(),
  costBasis: z.enum(["per task", "per run"]).nullable(),
  tokens: nonNegative.nullable(),
  modelReleaseDate: isoDate.nullable(),
  /** When the evaluation ran, when the source says. */
  runDate: isoDate.nullable(),
  /** When the source published the row or the board. */
  publishedDate: isoDate.nullable(),
  sourceUrl: httpsUrl,
  notes: z.string().nullable(),
});
export type BenchmarkResult = z.infer<typeof resultSchema>;

export const benchmarkMetaSchema = z.object({
  /** Score a random guess would get, in the benchmark's unit. */
  randomBaseline: z.number().finite().nullable(),
  /** Highest score Epoch AI considers reachable (label noise etc.). */
  ceiling: z.number().finite().nullable(),
  releaseDate: isoDate.nullable(),
  supersededBy: z.string().nullable(),
});
export type BenchmarkMeta = z.infer<typeof benchmarkMetaSchema>;

export const benchmarkResultsSchema = z.object({
  slug: z.string().regex(/^[a-z0-9]+(?:-[a-z0-9]+)*$/),
  /** The benchmark's name or version as the source writes it. */
  sourceName: z.string().min(1),
  meta: benchmarkMetaSchema.nullable(),
  results: z.array(resultSchema),
});
export type BenchmarkResults = z.infer<typeof benchmarkResultsSchema>;

export const sourceSnapshotSchema = z.object({
  source: sourceIdSchema,
  /** When the sync fetched the data (UTC). */
  retrievedAt: isoDateTime,
  fetchUrls: z.array(httpsUrl).min(1),
  /** Newest date the source itself puts on the data (publish or run date). */
  dataDate: isoDate.nullable(),
  benchmarks: z.array(benchmarkResultsSchema).min(1),
});
export type SourceSnapshot = z.infer<typeof sourceSnapshotSchema>;

export const offeringSchema = z.object({
  /** models.dev provider id. */
  provider: z.string().min(1),
  /** The provider's model id. */
  id: z.string().min(1),
  /** USD per 1M tokens; null when the provider lists no price. */
  input: nonNegative.nullable(),
  output: nonNegative.nullable(),
  cacheRead: nonNegative.nullable(),
  cacheWrite: nonNegative.nullable(),
  context: nonNegative.nullable(),
  maxOutput: nonNegative.nullable(),
});
export type Offering = z.infer<typeof offeringSchema>;

export const modelSpecSchema = z.object({
  modelKey,
  name: z.string().min(1),
  /** models.dev provider id of the lab that makes the model, when listed there. */
  labProvider: z.string().nullable(),
  releaseDate: isoDate.nullable(),
  /** "2026-06" or "2026-06-30" as listed. */
  knowledgeCutoff: z.string().regex(/^\d{4}-\d{2}(?:-\d{2})?$/).nullable(),
  context: nonNegative.nullable(),
  maxOutput: nonNegative.nullable(),
  inputModalities: z.array(z.string()),
  outputModalities: z.array(z.string()),
  openWeights: z.boolean().nullable(),
  reasoning: z.boolean().nullable(),
  toolCall: z.boolean().nullable(),
  /** First entry is the reference list price (the lab's own API when listed). */
  offerings: z.array(offeringSchema),
});
export type ModelSpec = z.infer<typeof modelSpecSchema>;

export const catalogSnapshotSchema = z.object({
  source: z.literal("models.dev"),
  retrievedAt: isoDateTime,
  fetchUrls: z.array(httpsUrl).min(1),
  providers: z.record(z.string(), z.object({ name: z.string().min(1), url: httpsUrl.nullable() })),
  models: z.array(modelSpecSchema),
});
export type CatalogSnapshot = z.infer<typeof catalogSnapshotSchema>;

export const aliasRuleSchema = z.object({
  source: z.enum([...SOURCE_IDS, "models.dev"]),
  externalId: z.string().min(1),
  modelKey: modelKey.nullable(),
  method: mappingMethodSchema,
  reviewed: z.boolean(),
  note: z.string().optional(),
});
export const aliasFileSchema = z.object({ aliases: z.array(aliasRuleSchema) });
export type AliasFile = z.infer<typeof aliasFileSchema>;

export const reportSchema = z.object({
  generatedAt: isoDateTime,
  sources: z.array(
    z.object({
      source: z.enum([...SOURCE_IDS, "models.dev"]),
      ok: z.boolean(),
      /** Set when the fetch failed and the previous snapshot was kept. */
      error: z.string().nullable(),
      results: z.number().int().nonnegative(),
      mappedResults: z.number().int().nonnegative(),
      externalModels: z.number().int().nonnegative(),
      mappedModels: z.number().int().nonnegative(),
    }),
  ),
  methods: z.object({ exact: z.number().int(), normalized: z.number().int(), manual: z.number().int(), unreviewed: z.number().int() }),
  unmapped: z.array(
    z.object({
      source: z.enum([...SOURCE_IDS, "models.dev"]),
      externalModel: z.string(),
      name: z.string().nullable(),
      benchmarks: z.array(z.string()),
      newestDate: isoDate.nullable(),
    }),
  ),
});
export type SyncReport = z.infer<typeof reportSchema>;
