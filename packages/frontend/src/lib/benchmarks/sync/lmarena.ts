/**
 * LMArena leaderboard dataset (huggingface.co/datasets/lmarena-ai/leaderboard-dataset,
 * CC BY 4.0), read through the Hugging Face datasets-server JSON API so no
 * parquet reader is needed. Only the "latest" split, overall category.
 */
import { agentClientKey, splitDisplayName } from "../conditions";
import type { BenchmarkResult, SourceSnapshot } from "../schema";
import { mapExternalModel, type SyncContext } from "./context";

export const LMARENA_DATASET = "lmarena-ai/leaderboard-dataset";
export const LMARENA_DATASET_URL = `https://huggingface.co/datasets/${LMARENA_DATASET}`;
export const LMARENA_PAGE_SIZE = 100;

/** Benchmark slug -> dataset config. "text" is fetched for coverage only. */
export const LMARENA_BOARDS = { "webdev-arena": "webdev", "agent-arena": "agent" } as const;
export type ArenaBoardSlug = keyof typeof LMARENA_BOARDS;

export function lmarenaViewerUrl(config: string): string {
  return `${LMARENA_DATASET_URL}/viewer/${config}/latest`;
}

/** datasets-server /filter URL for one page of a board's overall rows. */
export function lmarenaFilterUrl(config: string, offset: number, length: number = LMARENA_PAGE_SIZE): string {
  const params = new URLSearchParams({
    dataset: LMARENA_DATASET,
    config,
    split: "latest",
    where: `"category"='overall'`,
    offset: String(offset),
    length: String(length),
  });
  return `https://datasets-server.huggingface.co/filter?${params.toString()}`;
}

export interface ArenaRow {
  model_name: string;
  organization?: string | null;
  license?: string | null;
  /** Bradley-Terry boards. */
  rating?: number | null;
  rating_lower?: number | null;
  rating_upper?: number | null;
  vote_count?: number | null;
  /** Agent Arena (IPS) boards. */
  score?: number | null;
  score_ci_lower?: number | null;
  score_ci_upper?: number | null;
  observation_count?: number | null;
  session_count?: number | null;
  rank?: number | null;
  category?: string | null;
  leaderboard_publish_date?: string | null;
}

export interface ArenaPage {
  rows: Array<{ row: ArenaRow }>;
  num_rows_total: number;
}

function finiteOrNull(value: number | null | undefined): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function isoOrNull(value: string | null | undefined): string | null {
  return value && /^\d{4}-\d{2}-\d{2}$/.test(value) ? value : null;
}

/** Arena names are display names ("Claude Opus 5.5 (Max)") or raw ids ("gpt-4o-2024-05-13"). */
function looksLikeId(name: string): boolean {
  return !/\s/.test(name);
}

export function parseArenaRows(slug: ArenaBoardSlug, rows: readonly ArenaRow[], ctx: SyncContext): BenchmarkResult[] {
  const config = LMARENA_BOARDS[slug];
  const out: BenchmarkResult[] = [];
  for (const row of rows) {
    if (row.category && row.category !== "overall") continue;
    const raw = row.model_name?.trim();
    const value = finiteOrNull(row.rating) ?? finiteOrNull(row.score);
    if (!raw || value === null) continue;
    const split = splitDisplayName(raw);
    const published = isoOrNull(row.leaderboard_publish_date);
    const resolution = mapExternalModel(
      ctx,
      "lmarena",
      { externalId: split.name, ids: looksLikeId(split.name) ? [split.name] : [], names: [raw], org: row.organization ?? null },
      { benchmark: slug, name: raw, date: published },
    );
    const rank = finiteOrNull(row.rank);
    out.push({
      externalId: raw,
      externalModel: split.name,
      name: raw,
      modelKey: resolution.key,
      mapping: resolution.method,
      reviewed: resolution.reviewed,
      effort: split.effort,
      thinkingBudget: split.thinkingBudget,
      agent: split.agent ?? (slug === "agent-arena" ? "Arena Agent Mode" : null),
      agentKey: agentClientKey(split.agent),
      score: value,
      ciLow: finiteOrNull(row.rating_lower) ?? finiteOrNull(row.score_ci_lower),
      ciHigh: finiteOrNull(row.rating_upper) ?? finiteOrNull(row.score_ci_upper),
      stderr: null,
      votes: finiteOrNull(row.vote_count),
      sessions: finiteOrNull(row.session_count),
      rank: rank !== null && Number.isInteger(rank) && rank > 0 ? rank : null,
      costUsd: null,
      costBasis: null,
      tokens: null,
      modelReleaseDate: null,
      runDate: null,
      publishedDate: published,
      sourceUrl: lmarenaViewerUrl(config),
      notes: null,
    });
  }
  return out;
}

export function buildArenaSnapshot(
  boards: Record<ArenaBoardSlug, readonly ArenaRow[]>,
  ctx: SyncContext,
  retrievedAt: string,
  fetchUrls: string[],
): SourceSnapshot {
  const benchmarks = (Object.keys(LMARENA_BOARDS) as ArenaBoardSlug[]).map((slug) => ({
    slug,
    sourceName: LMARENA_BOARDS[slug],
    meta: null,
    results: parseArenaRows(slug, boards[slug], ctx),
  }));
  let dataDate: string | null = null;
  for (const result of benchmarks.flatMap((b) => b.results)) {
    if (result.publishedDate && (dataDate === null || result.publishedDate > dataDate)) dataDate = result.publishedDate;
  }
  return { source: "lmarena", retrievedAt, fetchUrls, dataDate, benchmarks };
}
