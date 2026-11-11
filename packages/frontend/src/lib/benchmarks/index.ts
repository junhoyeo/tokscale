/**
 * Benchmarks, specs and list prices for the Models pages.
 *
 * Snapshots live in src/content/benchmarks (written by
 * scripts/benchmarks/sync.ts, validated with zod at load). Server code
 * imports from "@/lib/benchmarks"; client components receive plain rows.
 */
import { benchmarkContent } from "./content";
import { createBenchmarkModel } from "./model";

export const benchmarkData = createBenchmarkModel(benchmarkContent);

export * from "./definitions";
export * from "./format";
export * from "./urls";
export type { BenchmarkModel, ModelBenchmarkEntry, ResolvedBenchmark, SnapshotInfo } from "./model";
export type { BenchmarkResult, ModelSpec, Offering, SourceId } from "./schema";
export type { Effort } from "./conditions";
export { effortLabel } from "./conditions";
