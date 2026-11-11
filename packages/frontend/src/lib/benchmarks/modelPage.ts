/**
 * Plain data for the Benchmarks, Pricing and Specs sections of
 * /models/[key]. Pure: the benchmark model, the model's agent shares and the
 * token mixes are passed in, so the server page and the tests share one
 * implementation. Client components receive the result as props.
 */
import { agentRef } from "@/lib/explore/modelMeta";
import { benchmarkInfo, sourceNotes, type BenchmarkInfo, type SourceNote } from "./attribution";
import { DASHBOARD_BENCHMARKS } from "./columns";
import { scoreCell, type ModelMixes, type ScoreCell } from "./dashboard";
import { CATEGORY_LABELS } from "./definitions";
import { conditionTag, formatInterval, formatScore } from "./format";
import type { BenchmarkModel, ResolvedBenchmark } from "./model";
import { effectivePricePerMillion, mixShares, mixTotal, threeToOneBlend, type MixCounts } from "./price";
import type { BenchmarkResult, Offering } from "./schema";

/** One agent's share of this model's Tokscale tokens in the period (as in ModelDetail.agents). */
export interface AgentTokenShare {
  key: string;
  name: string;
  logoUrl: string | null;
  /** Share of the model's tokens that went through this agent (0..1). */
  share: number;
  activeUsers: number;
}

/** Tokscale usage of the model through the agent that ran a result. Shown next to the score, never as its cause. */
export interface AgentUsage {
  key: string;
  name: string;
  logoUrl: string | null;
  /** Share of the model's Tokscale tokens through this agent in the period; 0 when there were none. */
  share: number;
  activeUsers: number;
}

export interface ModelResultView {
  /** The source's id for the row; unique within one benchmark. */
  id: string;
  cell: ScoreCell;
  /** "Max · Claude Code"; null when the source reports no conditions. */
  conditions: string | null;
  /** Set when the result names an agent Tokscale tracks and the page has usage for the model. */
  agentUsage: AgentUsage | null;
}

export interface ModelBenchmarkView {
  slug: string;
  info: BenchmarkInfo;
  categoryLabel: string;
  /** Kept for context only (status "history"). */
  history: boolean;
  statusNote: string | null;
  /** The model's best result: the one the Models dashboard shows. */
  best: ModelResultView;
  /** Rank of the best result among the models matched in this benchmark (1 = top). */
  rank: number;
  rankedModels: number;
  publisher: string;
  /** Who collected the result, when that is not the publisher. */
  collectedBy: string | null;
  /** The model's other results (other efforts, agents or harnesses), best first. */
  others: ModelResultView[];
  /** Saturation or staleness notes for the benchmark. */
  notes: string[];
}

export interface ModelBenchmarksView {
  rows: ModelBenchmarkView[];
  /** Models dashboard benchmarks without a result for this model: "Not evaluated", never zero. */
  notEvaluated: Array<{ slug: string; name: string }>;
}

export interface OfferingView {
  provider: string;
  providerName: string;
  /** The provider's own model id. */
  modelId: string;
  /** USD per 1M tokens; null when the provider lists no price. */
  input: number | null;
  output: number | null;
  cacheRead: number | null;
  cacheWrite: number | null;
  context: number | null;
  /** The first offering: the price every estimate on the page uses. */
  reference: boolean;
}

/** Effective USD per 1M tokens of the reference list price at one token mix. */
export type MixEstimate =
  | { status: "ok"; perMillion: number; cacheReadShare: number; assumptions: string[] }
  /** No Tokscale usage data is available (no database). */
  | { status: "no-data" }
  /** The mix is empty: the model had no tokens in the period. */
  | { status: "no-usage" }
  /** The reference offering lacks a price the mix needs (or there is no offering). */
  | { status: "no-price" };

export interface ModelPricingView {
  offerings: OfferingView[];
  /** Whose list price is the reference; null when no provider lists a price. */
  reference: { providerName: string; isLab: boolean } | null;
  /** At the token mix of every identified model in the period. */
  community: MixEstimate;
  /** At this model's own token mix; null on pages for models without Tokscale usage. */
  own: MixEstimate | null;
  /** The usual 3:1 input/output blend of the reference price, for comparison. */
  blend: number | null;
  /** Every assumption behind the estimates, in plain sentences, without duplicates. */
  assumptions: string[];
}

export interface ModelSpecsView {
  releaseDate: string | null;
  knowledgeCutoff: string | null;
  context: number | null;
  maxOutput: number | null;
  inputModalities: string[];
  outputModalities: string[];
  openWeights: boolean | null;
  reasoning: boolean | null;
  toolCall: boolean | null;
  /** When the models.dev catalog was retrieved (ISO). */
  retrievedAt: string;
}

export interface ModelResearchView {
  benchmarks: ModelBenchmarksView;
  /** Null when models.dev does not list the model. */
  pricing: ModelPricingView | null;
  /** Null when models.dev does not list the model. */
  specs: ModelSpecsView | null;
  /** Sources behind the numbers on the page, for the sources footer. */
  sources: SourceNote[];
}

export interface ModelResearchOptions {
  /** The model's Tokscale usage in the period; null for models without usage. */
  usage: { agents: readonly AgentTokenShare[] } | null;
  /** Token mixes for the period; null when Tokscale usage data is unavailable. */
  mixes: ModelMixes | null;
}

/** Assumptions behind every effective price, whatever the mix. */
export const PRICE_ASSUMPTIONS: readonly string[] = [
  "This is an estimate at list prices, not what anyone paid.",
  "Reasoning tokens are priced as output.",
  "Long-context tiers, batch pricing and discounts are not applied.",
  "Token mixes come from public Tokscale submissions. Hidden accounts are excluded.",
];

/** True when Tokscale has benchmark results or a models.dev entry for the key. */
export function hasModelResearch(model: BenchmarkModel, key: string): boolean {
  return model.spec(key) !== null || model.forModel(key).length > 0;
}

/** What a page for a model without Tokscale usage can show. */
export interface ResearchParts {
  /** At least one tracked benchmark has a result for the model. */
  benchmarks: boolean;
  /** models.dev lists the model (list prices and specs). */
  catalog: boolean;
}

export function researchParts(model: BenchmarkModel, key: string): ResearchParts {
  return { benchmarks: model.forModel(key).length > 0, catalog: model.spec(key) !== null };
}

/**
 * Names for what the page shows, so titles and notes never promise a
 * section that is empty: a title part ("Benchmarks, List Prices, Specs")
 * and a phrase ("benchmark results, list prices and specs").
 */
export function researchLabel(parts: ResearchParts): { title: string; phrase: string } {
  if (parts.benchmarks && parts.catalog) return { title: "Benchmarks, List Prices, Specs", phrase: "benchmark results, list prices and specs" };
  if (parts.benchmarks) return { title: "Benchmarks", phrase: "benchmark results" };
  return { title: "List Prices, Specs", phrase: "list prices and specs" };
}

function cellDate(r: BenchmarkResult): Pick<ScoreCell, "date" | "dateKind"> {
  if (r.runDate) return { date: r.runDate, dateKind: "run" };
  if (r.publishedDate) return { date: r.publishedDate, dateKind: "published" };
  if (r.modelReleaseDate) return { date: r.modelReleaseDate, dateKind: "release" };
  return { date: null, dateKind: null };
}

/** A score cell for any one result (the dashboard's scoreCell only builds the best). */
export function resultScoreCell(benchmark: ResolvedBenchmark, result: BenchmarkResult): ScoreCell {
  const unit = benchmark.definition.unit;
  return {
    value: result.score,
    text: formatScore(unit, result.score),
    tag: conditionTag(result.effort, result.agent, result.thinkingBudget),
    interval: formatInterval(unit, result.ciLow, result.ciHigh),
    sourceName: result.name ?? result.externalId,
    ...cellDate(result),
    sourceUrl: result.sourceUrl,
    resultCount: 1,
  };
}

/**
 * Tokscale usage of the model through the agent that ran `result`. Null when
 * the result names no tracked agent or the page has no usage for the model.
 * An agent with no tokens for the model in the period gets share 0.
 */
export function agentUsageFor(result: BenchmarkResult, usage: ModelResearchOptions["usage"]): AgentUsage | null {
  if (!usage || !result.agentKey) return null;
  const agent = usage.agents.find((a) => a.key === result.agentKey);
  if (agent) {
    return { key: agent.key, name: agent.name, logoUrl: agent.logoUrl, share: agent.share, activeUsers: agent.activeUsers };
  }
  const ref = agentRef(result.agentKey);
  const name = ref.name !== ref.key ? ref.name : (result.agent ?? ref.name);
  return { key: ref.key, name, logoUrl: ref.logoUrl, share: 0, activeUsers: 0 };
}

function resultView(result: BenchmarkResult, cell: ScoreCell, usage: ModelResearchOptions["usage"]): ModelResultView {
  return {
    id: result.externalId,
    cell,
    conditions: conditionTag(result.effort, result.agent, result.thinkingBudget),
    agentUsage: agentUsageFor(result, usage),
  };
}

export function buildModelBenchmarks(
  model: BenchmarkModel,
  key: string,
  usage: ModelResearchOptions["usage"],
): ModelBenchmarksView {
  const rows: ModelBenchmarkView[] = model.forModel(key).map((entry) => {
    const { benchmark, best } = entry;
    const definition = benchmark.definition;
    const bestCell = scoreCell(model, definition.slug, key) ?? resultScoreCell(benchmark, best);
    return {
      slug: definition.slug,
      info: benchmarkInfo(benchmark),
      categoryLabel: CATEGORY_LABELS[definition.category],
      history: definition.status === "history",
      statusNote: definition.statusNote,
      best: resultView(best, bestCell, usage),
      rank: entry.rank,
      rankedModels: entry.rankedModels,
      publisher: definition.publisher,
      collectedBy: definition.collectedBy,
      others: entry.results
        .filter((r) => r !== best)
        .map((r) => resultView(r, resultScoreCell(benchmark, r), usage)),
      notes: [benchmark.saturationNote, benchmark.staleNote].filter((note): note is string => note !== null),
    };
  });
  const evaluated = new Set(rows.map((row) => row.slug));
  const notEvaluated = DASHBOARD_BENCHMARKS.filter((slug) => !evaluated.has(slug)).flatMap((slug) => {
    const benchmark = model.get(slug);
    return benchmark ? [{ slug, name: benchmark.definition.name }] : [];
  });
  return { rows, notEvaluated };
}

/** Effective price of `offering` at `mix`, or the reason there is none. */
export function mixEstimate(offering: Offering | null, mix: MixCounts | null | undefined, hasData: boolean): MixEstimate {
  if (!hasData) return { status: "no-data" };
  if (!mix || mixTotal(mix) <= 0) return { status: "no-usage" };
  const price = offering ? effectivePricePerMillion(offering, mix) : null;
  if (!price) return { status: "no-price" };
  return { status: "ok", perMillion: price.perMillion, cacheReadShare: mixShares(mix).cacheRead, assumptions: price.assumptions };
}

export function buildModelPricing(model: BenchmarkModel, key: string, options: ModelResearchOptions): ModelPricingView | null {
  const spec = model.spec(key);
  if (!spec) return null;
  const reference = spec.offerings[0] ?? null;
  const community = mixEstimate(reference, options.mixes?.community, options.mixes !== null);
  const own = options.usage ? mixEstimate(reference, options.mixes?.byModel[key], options.mixes !== null) : null;
  const assumptions = [...PRICE_ASSUMPTIONS];
  for (const estimate of [own, community]) {
    if (estimate?.status !== "ok") continue;
    for (const line of estimate.assumptions) if (!assumptions.includes(line)) assumptions.push(line);
  }
  return {
    offerings: spec.offerings.map((offering, index) => ({
      provider: offering.provider,
      providerName: model.provider(offering.provider)?.name ?? offering.provider,
      modelId: offering.id,
      input: offering.input,
      output: offering.output,
      cacheRead: offering.cacheRead,
      cacheWrite: offering.cacheWrite,
      context: offering.context,
      reference: index === 0,
    })),
    reference: reference
      ? {
          providerName: model.provider(reference.provider)?.name ?? reference.provider,
          isLab: spec.labProvider !== null && reference.provider === spec.labProvider,
        }
      : null,
    community,
    own,
    blend: reference ? threeToOneBlend(reference) : null,
    assumptions,
  };
}

export function buildModelSpecs(model: BenchmarkModel, key: string): ModelSpecsView | null {
  const spec = model.spec(key);
  if (!spec) return null;
  return {
    releaseDate: spec.releaseDate,
    knowledgeCutoff: spec.knowledgeCutoff,
    context: spec.context,
    maxOutput: spec.maxOutput,
    inputModalities: [...spec.inputModalities],
    outputModalities: [...spec.outputModalities],
    openWeights: spec.openWeights,
    reasoning: spec.reasoning,
    toolCall: spec.toolCall,
    retrievedAt: model.catalogInfo().retrievedAt,
  };
}

/** Everything the Benchmarks, Pricing, Specs and Sources sections show for one model. */
export function buildModelResearch(model: BenchmarkModel, key: string, options: ModelResearchOptions): ModelResearchView {
  const benchmarks = buildModelBenchmarks(model, key, options.usage);
  const pricing = buildModelPricing(model, key, options);
  const specs = buildModelSpecs(model, key);
  return {
    benchmarks,
    pricing,
    specs,
    sources: sourceNotes(model, { slugs: benchmarks.rows.map((row) => row.slug), includeCatalog: specs !== null }),
  };
}
