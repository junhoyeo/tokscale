/**
 * Rows for the /models dashboard: Tokscale usage, benchmark scores, list
 * prices, the effective price at real agent usage, and specs, joined on the
 * canonical model key. Pure: usage and token mixes are passed in.
 */
import { modelRef } from "@/lib/explore/modelMeta";
import type { ModelUsageRow } from "@/lib/explore/types";
import { DASHBOARD_BENCHMARKS } from "./columns";
import { conditionTag, formatInterval, formatScore } from "./format";
import type { BenchmarkModel } from "./model";
import { effectivePricePerMillion, type EffectivePrice, type MixCounts } from "./price";
import type { BenchmarkResult } from "./schema";

/** Default rows: models with this many active users in the period... */
export const DASHBOARD_MIN_USERS = 5;
/** ...plus the top models by the Epoch Capabilities Index. */
export const DASHBOARD_TOP_ECI = 20;
export const ECI_SLUG = "epoch-capabilities-index";

export interface ScoreCell {
  value: number;
  text: string;
  /** "Max · Claude Code" */
  tag: string | null;
  interval: string | null;
  /** Name of the entry in the source, e.g. "Claude Opus 5.5 (Max)". */
  sourceName: string;
  date: string | null;
  dateKind: "run" | "published" | "release" | null;
  sourceUrl: string;
  /** Results this model has in the benchmark (efforts, agents). */
  resultCount: number;
}

export interface PriceCell {
  provider: string;
  providerName: string;
  input: number | null;
  output: number | null;
  cacheRead: number | null;
  cacheWrite: number | null;
  /** Effective $/1M at the community's token mix in the period. */
  community: EffectivePrice | null;
  /** Effective $/1M at this model's own token mix in the period. */
  own: EffectivePrice | null;
}

export interface SpecCell {
  context: number | null;
  maxOutput: number | null;
  releaseDate: string | null;
  knowledgeCutoff: string | null;
  openWeights: boolean | null;
  inputModalities: string[];
}

export interface DashboardRow {
  key: string;
  name: string;
  lab: string | null;
  logoUrl: string | null;
  usage: {
    activeUsers: number;
    previousActiveUsers: number | null;
    isNew: boolean;
    tokens: number;
    adoption: number;
    topAgent: { key: string; name: string; logoUrl: string | null; share: number } | null;
  } | null;
  scores: Record<string, ScoreCell | null>;
  price: PriceCell | null;
  spec: SpecCell | null;
}

export interface ModelMixes {
  /** Sum of every identified model's tokens in the period. */
  community: MixCounts;
  byModel: Record<string, MixCounts>;
}

function cellDate(r: BenchmarkResult): Pick<ScoreCell, "date" | "dateKind"> {
  if (r.runDate) return { date: r.runDate, dateKind: "run" };
  if (r.publishedDate) return { date: r.publishedDate, dateKind: "published" };
  if (r.modelReleaseDate) return { date: r.modelReleaseDate, dateKind: "release" };
  return { date: null, dateKind: null };
}

export function scoreCell(model: BenchmarkModel, slug: string, key: string): ScoreCell | null {
  const benchmark = model.get(slug);
  const best = model.best(slug).get(key);
  if (!benchmark || !best) return null;
  const unit = benchmark.definition.unit;
  return {
    value: best.score,
    text: formatScore(unit, best.score),
    tag: conditionTag(best.effort, best.agent, best.thinkingBudget),
    interval: formatInterval(unit, best.ciLow, best.ciHigh),
    sourceName: best.name ?? best.externalId,
    ...cellDate(best),
    sourceUrl: best.sourceUrl,
    resultCount: benchmark.results.filter((r) => r.modelKey === key).length,
  };
}

export function priceCell(model: BenchmarkModel, key: string, mixes: ModelMixes | null): PriceCell | null {
  const offering = model.referenceOffering(key);
  if (!offering) return null;
  const own = mixes?.byModel[key];
  return {
    provider: offering.provider,
    providerName: model.provider(offering.provider)?.name ?? offering.provider,
    input: offering.input,
    output: offering.output,
    cacheRead: offering.cacheRead,
    cacheWrite: offering.cacheWrite,
    community: mixes ? effectivePricePerMillion(offering, mixes.community) : null,
    own: own ? effectivePricePerMillion(offering, own) : null,
  };
}

export function specCell(model: BenchmarkModel, key: string): SpecCell | null {
  const spec = model.spec(key);
  if (!spec) return null;
  return {
    context: spec.context,
    maxOutput: spec.maxOutput,
    releaseDate: spec.releaseDate,
    knowledgeCutoff: spec.knowledgeCutoff,
    openWeights: spec.openWeights,
    inputModalities: spec.inputModalities,
  };
}

/**
 * Keys for the default rows: models with at least `minUsers` active users
 * in the period (by active users), then the top `topEci` models by ECI that
 * are not already listed (by ECI).
 */
export function selectDashboardKeys(
  usageRows: readonly ModelUsageRow[],
  eciBest: ReadonlyMap<string, BenchmarkResult>,
  minUsers: number = DASHBOARD_MIN_USERS,
  topEci: number = DASHBOARD_TOP_ECI,
): string[] {
  const keys = usageRows
    .filter((r) => r.activeUsers >= minUsers)
    .sort((a, b) => b.activeUsers - a.activeUsers || b.tokens - a.tokens || (a.key < b.key ? -1 : 1))
    .map((r) => r.key);
  const seen = new Set(keys);
  const byEci = [...eciBest.entries()].sort((a, b) => b[1].score - a[1].score || (a[0] < b[0] ? -1 : 1)).slice(0, topEci);
  for (const [key] of byEci) {
    if (!seen.has(key)) {
      keys.push(key);
      seen.add(key);
    }
  }
  return keys;
}

export function buildDashboardRow(
  model: BenchmarkModel,
  key: string,
  usage: ModelUsageRow | null,
  mixes: ModelMixes | null,
): DashboardRow {
  const ref = usage ?? modelRef(key);
  const scores: Record<string, ScoreCell | null> = {};
  for (const slug of DASHBOARD_BENCHMARKS) scores[slug] = scoreCell(model, slug, key);
  const top = usage?.topAgents[0] ?? null;
  return {
    key,
    name: ref.name,
    lab: ref.lab,
    logoUrl: ref.logoUrl,
    usage: usage
      ? {
          activeUsers: usage.activeUsers,
          previousActiveUsers: usage.previousActiveUsers,
          isNew: usage.isNew,
          tokens: usage.tokens,
          adoption: usage.adoption,
          topAgent: top ? { key: top.key, name: top.name, logoUrl: top.logoUrl, share: top.share } : null,
        }
      : null,
    scores,
    price: priceCell(model, key, mixes),
    spec: specCell(model, key),
  };
}

export function buildDashboardRows(
  model: BenchmarkModel,
  usageRows: readonly ModelUsageRow[],
  mixes: ModelMixes | null,
  options: { minUsers?: number; topEci?: number } = {},
): DashboardRow[] {
  const usageByKey = new Map(usageRows.map((r) => [r.key, r] as const));
  const keys = selectDashboardKeys(usageRows, model.best(ECI_SLUG), options.minUsers, options.topEci);
  return keys.map((key) => buildDashboardRow(model, key, usageByKey.get(key) ?? null, mixes));
}

/** Sort rows by a column. Missing values always go last; ties keep the incoming order. */
export function sortDashboardRows(rows: readonly DashboardRow[], sort: string, ascending: boolean): DashboardRow[] {
  const value = (row: DashboardRow): number | null => {
    if (sort === "users") return row.usage?.activeUsers ?? null;
    if (sort === "tokens") return row.usage?.tokens ?? null;
    if (sort === "price-community") return row.price?.community?.perMillion ?? null;
    if (sort === "price-own") return row.price?.own?.perMillion ?? null;
    if (sort === "price-input") return row.price?.input ?? null;
    if (sort === "context") return row.spec?.context ?? null;
    if (sort === "released") return row.spec?.releaseDate ? Date.parse(`${row.spec.releaseDate}T00:00:00Z`) : null;
    return row.scores[sort]?.value ?? null;
  };
  return rows
    .map((row, index) => ({ row, index, v: value(row) }))
    .sort((a, b) => {
      if (a.v === null && b.v === null) return a.index - b.index;
      if (a.v === null) return 1;
      if (b.v === null) return -1;
      const diff = ascending ? a.v - b.v : b.v - a.v;
      return diff !== 0 ? diff : a.index - b.index;
    })
    .map((entry) => entry.row);
}
