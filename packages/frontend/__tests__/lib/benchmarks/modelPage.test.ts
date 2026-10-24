import { describe, expect, it } from "vitest";
import { DASHBOARD_BENCHMARKS } from "@/lib/benchmarks/columns";
import type { BenchmarkContent } from "@/lib/benchmarks/content";
import { scoreCell, type ModelMixes } from "@/lib/benchmarks/dashboard";
import { getBenchmarkDefinition } from "@/lib/benchmarks/definitions";
import { createBenchmarkModel } from "@/lib/benchmarks/model";
import {
  agentUsageFor,
  buildModelBenchmarks,
  buildModelPricing,
  buildModelResearch,
  buildModelSpecs,
  hasModelResearch,
  mixEstimate,
  PRICE_ASSUMPTIONS,
  researchLabel,
  researchParts,
  resultScoreCell,
  type AgentTokenShare,
} from "@/lib/benchmarks/modelPage";
import { effectivePricePerMillion, threeToOneBlend, type MixCounts } from "@/lib/benchmarks/price";
import type { BenchmarkResult, ModelSpec, Offering, SourceSnapshot } from "@/lib/benchmarks/schema";

function result(partial: Partial<BenchmarkResult> & Pick<BenchmarkResult, "externalId" | "score">): BenchmarkResult {
  return {
    externalModel: partial.externalId,
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
    sourceUrl: "https://example.com/",
    notes: null,
    ...partial,
  };
}

const mapped = (key: string, externalId: string, score: number, extra: Partial<BenchmarkResult> = {}) =>
  result({ externalId, score, modelKey: key, mapping: "exact", ...extra });

function offering(provider: string, prices: Partial<Offering> = {}): Offering {
  return { provider, id: `${provider}/model`, input: 4, output: 20, cacheRead: 0.2, cacheWrite: 5, context: 1_000_000, maxOutput: 128_000, ...prices };
}

function spec(modelKey: string, partial: Partial<ModelSpec> = {}): ModelSpec {
  return {
    modelKey,
    name: modelKey,
    labProvider: "anthropic",
    releaseDate: "2026-09-22",
    knowledgeCutoff: "2026-06",
    context: 1_000_000,
    maxOutput: 128_000,
    inputModalities: ["text", "image"],
    outputModalities: ["text"],
    openWeights: false,
    reasoning: true,
    toolCall: true,
    offerings: [offering("anthropic"), offering("bedrock")],
    ...partial,
  };
}

function content(): BenchmarkContent {
  const epoch: SourceSnapshot = {
    source: "epoch",
    retrievedAt: "2026-10-21T01:00:00.000Z",
    fetchUrls: ["https://epoch.ai/data/benchmark_data.zip"],
    dataDate: "2026-10-17",
    benchmarks: [
      {
        slug: "epoch-capabilities-index",
        sourceName: "Epoch Capabilities Index",
        meta: null,
        results: [mapped("opus", "Opus", 168.8, { modelReleaseDate: "2026-09-22" }), mapped("sol", "Sol", 168.4), mapped("bench-only", "Bench only", 150)],
      },
      {
        slug: "gpqa-diamond",
        sourceName: "GPQA diamond",
        meta: { randomBaseline: 0.25, ceiling: 0.97, releaseDate: "2023-11-20", supersededBy: null },
        results: [
          mapped("opus", "opus_high", 0.9, { effort: "high", runDate: "2026-10-01" }),
          mapped("opus", "opus_max", 0.95, { effort: "max", runDate: "2026-10-02" }),
          mapped("sol", "sol_max", 0.96, { effort: "max", runDate: "2026-10-03" }),
        ],
      },
    ],
  };
  const tbench: SourceSnapshot = {
    source: "tbench",
    retrievedAt: "2026-10-21T01:00:00.000Z",
    fetchUrls: ["https://www.tbench.ai/leaderboard"],
    dataDate: "2026-10-06",
    benchmarks: [
      {
        slug: "terminal-bench-4",
        sourceName: "Terminal-Bench 4.0",
        meta: null,
        results: [
          mapped("opus", "Claude Code / Opus", 0.6485, {
            name: "Opus 5.5",
            agent: "Claude Code",
            agentKey: "claude",
            effort: "max",
            publishedDate: "2026-10-06",
            ciLow: 0.6174,
            ciHigh: 0.6796,
          }),
          mapped("opus", "Codex / Opus", 0.5, { agent: "Codex", agentKey: "codex", effort: "high", publishedDate: "2026-09-01" }),
        ],
      },
    ],
  };
  return {
    sources: new Map([
      ["epoch", epoch],
      ["tbench", tbench],
    ]),
    catalog: {
      source: "models.dev",
      retrievedAt: "2026-10-21T01:00:00.000Z",
      fetchUrls: ["https://models.dev/api.json"],
      providers: {
        anthropic: { name: "Anthropic", url: null },
        openrouter: { name: "OpenRouter", url: null },
      },
      models: [
        spec("opus"),
        // A reseller price only: the reference is the first provider, not the lab.
        spec("spec-only", { labProvider: "acme", offerings: [offering("openrouter", { cacheWrite: null }), offering("unknown-host")] }),
        spec("no-offers", { offerings: [] }),
      ],
    },
    report: { generatedAt: "2026-10-21T01:00:00.000Z", sources: [], methods: { exact: 0, normalized: 0, manual: 0, unreviewed: 0 }, unmapped: [] },
  };
}

const model = createBenchmarkModel(content());

function mix(partial: Partial<MixCounts>): MixCounts {
  return { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, reasoning: 0, ...partial };
}

const MIXES: ModelMixes = {
  community: mix({ input: 10, output: 10, cacheRead: 970, cacheWrite: 10 }),
  byModel: { opus: mix({ input: 5, output: 5, cacheRead: 980, cacheWrite: 10 }) },
};

const AGENTS: AgentTokenShare[] = [{ key: "claude", name: "Claude Code", logoUrl: "/claude.png", share: 0.9, activeUsers: 146 }];

describe("hasModelResearch", () => {
  it("is true for a models.dev entry or a matched result, false otherwise", () => {
    expect(hasModelResearch(model, "opus")).toBe(true);
    expect(hasModelResearch(model, "spec-only")).toBe(true);
    expect(hasModelResearch(model, "bench-only")).toBe(true);
    expect(hasModelResearch(model, "nobody")).toBe(false);
  });
});

describe("researchParts and researchLabel", () => {
  it("names only what the page can show", () => {
    expect(researchParts(model, "opus")).toEqual({ benchmarks: true, catalog: true });
    expect(researchParts(model, "bench-only")).toEqual({ benchmarks: true, catalog: false });
    expect(researchParts(model, "spec-only")).toEqual({ benchmarks: false, catalog: true });
    expect(researchLabel({ benchmarks: true, catalog: true })).toEqual({
      title: "Benchmarks, List Prices, Specs",
      phrase: "benchmark results, list prices and specs",
    });
    expect(researchLabel({ benchmarks: true, catalog: false })).toEqual({ title: "Benchmarks", phrase: "benchmark results" });
    expect(researchLabel({ benchmarks: false, catalog: true })).toEqual({ title: "List Prices, Specs", phrase: "list prices and specs" });
  });
});

describe("resultScoreCell", () => {
  it("formats one result with its conditions, interval and date kind", () => {
    const tb = model.get("terminal-bench-4");
    if (!tb) throw new Error("missing benchmark");
    const cell = resultScoreCell(tb, tb.results[0]);
    expect(cell).toMatchObject({
      value: 0.6485,
      text: "64.8%",
      tag: "Max \u00b7 Claude Code",
      interval: "61.7\u201368.0%",
      sourceName: "Opus 5.5",
      date: "2026-10-06",
      dateKind: "published",
      resultCount: 1,
    });
  });

  it("falls back to the external id and a release date", () => {
    const eci = model.get("epoch-capabilities-index");
    if (!eci) throw new Error("missing benchmark");
    const cell = resultScoreCell(eci, eci.results[0]);
    expect(cell.sourceName).toBe("Opus");
    expect(cell.text).toBe("168.8");
    expect(cell.tag).toBeNull();
    expect(cell.date).toBe("2026-09-22");
    expect(cell.dateKind).toBe("release");
  });
});

describe("agentUsageFor", () => {
  const tb = model.get("terminal-bench-4");
  if (!tb) throw new Error("missing benchmark");
  const [claudeRow, codexRow] = tb.results;

  it("is null without usage or without a tracked agent", () => {
    expect(agentUsageFor(claudeRow, null)).toBeNull();
    const gpqa = model.get("gpqa-diamond");
    expect(agentUsageFor(gpqa?.results[0] as BenchmarkResult, { agents: AGENTS })).toBeNull();
  });

  it("uses the model's share of tokens through that agent", () => {
    expect(agentUsageFor(claudeRow, { agents: AGENTS })).toEqual({
      key: "claude",
      name: "Claude Code",
      logoUrl: "/claude.png",
      share: 0.9,
      activeUsers: 146,
    });
  });

  it("reports a zero share, not a missing one, when the agent carried none of the model's tokens", () => {
    const usage = agentUsageFor(codexRow, { agents: AGENTS });
    expect(usage).toMatchObject({ key: "codex", name: "Codex CLI", share: 0, activeUsers: 0 });
  });
});

describe("buildModelBenchmarks", () => {
  const view = buildModelBenchmarks(model, "opus", { agents: AGENTS });

  it("lists every benchmark with a result in definition order, with ranks among matched models", () => {
    expect(view.rows.map((row) => row.slug)).toEqual(["epoch-capabilities-index", "terminal-bench-4", "gpqa-diamond"]);
    const eci = view.rows[0];
    expect(eci.rank).toBe(1);
    expect(eci.rankedModels).toBe(3);
    const gpqa = view.rows[2];
    expect(gpqa.rank).toBe(2);
    expect(gpqa.rankedModels).toBe(2);
  });

  it("shows the dashboard cell as the best result and the other runs best first", () => {
    const gpqa = view.rows.find((row) => row.slug === "gpqa-diamond");
    expect(gpqa?.best.cell).toEqual(scoreCell(model, "gpqa-diamond", "opus"));
    expect(gpqa?.best.cell.resultCount).toBe(2);
    expect(gpqa?.best.conditions).toBe("Max");
    expect(gpqa?.others.map((other) => other.id)).toEqual(["opus_high"]);
    expect(gpqa?.others[0].cell.resultCount).toBe(1);
    expect(gpqa?.notes[0]).toMatch(/^Near its ceiling/);
  });

  it("joins agent usage to agent rows only", () => {
    const tb = view.rows.find((row) => row.slug === "terminal-bench-4");
    expect(tb?.best.agentUsage?.share).toBe(0.9);
    expect(tb?.others[0].agentUsage?.share).toBe(0);
    expect(view.rows[0].best.agentUsage).toBeNull();
  });

  it("names the dashboard benchmarks without a result as not evaluated, in column order", () => {
    const expected = DASHBOARD_BENCHMARKS.filter((slug) => !["epoch-capabilities-index", "terminal-bench-4", "gpqa-diamond"].includes(slug));
    expect(view.notEvaluated.map((item) => item.slug)).toEqual(expected);
    expect(view.notEvaluated[0].name).toBe(getBenchmarkDefinition(expected[0])?.name);
  });

  it("has no rows and every dashboard benchmark as not evaluated for an unknown model", () => {
    const empty = buildModelBenchmarks(model, "nobody", null);
    expect(empty.rows).toEqual([]);
    expect(empty.notEvaluated).toHaveLength(DASHBOARD_BENCHMARKS.length);
  });
});

describe("mixEstimate", () => {
  const price = offering("anthropic");

  it("says why there is no estimate", () => {
    expect(mixEstimate(price, MIXES.community, false)).toEqual({ status: "no-data" });
    expect(mixEstimate(price, undefined, true)).toEqual({ status: "no-usage" });
    expect(mixEstimate(price, mix({}), true)).toEqual({ status: "no-usage" });
    expect(mixEstimate(null, MIXES.community, true)).toEqual({ status: "no-price" });
    expect(mixEstimate(offering("x", { output: null }), MIXES.community, true)).toEqual({ status: "no-price" });
  });

  it("weights the list price by the mix and reports the cache-read share", () => {
    const estimate = mixEstimate(price, MIXES.byModel.opus, true);
    expect(estimate.status).toBe("ok");
    if (estimate.status !== "ok") return;
    expect(estimate.perMillion).toBeCloseTo(effectivePricePerMillion(price, MIXES.byModel.opus)?.perMillion ?? Number.NaN, 12);
    // (5*4 + 5*20 + 980*0.2 + 10*5) / 1000
    expect(estimate.perMillion).toBeCloseTo(0.366, 12);
    expect(estimate.cacheReadShare).toBeCloseTo(0.98, 12);
    expect(estimate.assumptions).toEqual([]);
  });
});

describe("buildModelPricing", () => {
  it("is null without a models.dev entry", () => {
    expect(buildModelPricing(model, "bench-only", { usage: null, mixes: MIXES })).toBeNull();
  });

  it("uses the lab's own API as the reference and estimates both mixes", () => {
    const pricing = buildModelPricing(model, "opus", { usage: { agents: AGENTS }, mixes: MIXES });
    expect(pricing?.reference).toEqual({ providerName: "Anthropic", isLab: true });
    expect(pricing?.offerings.map((o) => [o.provider, o.providerName, o.reference])).toEqual([
      ["anthropic", "Anthropic", true],
      ["bedrock", "bedrock", false],
    ]);
    expect(pricing?.own?.status).toBe("ok");
    expect(pricing?.community.status).toBe("ok");
    expect(pricing?.blend).toBe(threeToOneBlend(offering("anthropic")));
    expect(pricing?.blend).toBe(8);
    expect(pricing?.assumptions).toEqual([...PRICE_ASSUMPTIONS]);
  });

  it("has no own-mix estimate on pages without usage and no estimate without usage data", () => {
    const noUsage = buildModelPricing(model, "opus", { usage: null, mixes: MIXES });
    expect(noUsage?.own).toBeNull();
    expect(noUsage?.community.status).toBe("ok");
    const noData = buildModelPricing(model, "opus", { usage: { agents: AGENTS }, mixes: null });
    expect(noData?.own).toEqual({ status: "no-data" });
    expect(noData?.community).toEqual({ status: "no-data" });
  });

  it("names a reseller reference and lists each fallback assumption once", () => {
    const pricing = buildModelPricing(model, "spec-only", { usage: null, mixes: MIXES });
    expect(pricing?.reference).toEqual({ providerName: "OpenRouter", isLab: false });
    expect(pricing?.offerings[1].providerName).toBe("unknown-host");
    const fallback = "No cache-write price is listed, so cache writes are priced as input.";
    expect(pricing?.assumptions.filter((line) => line === fallback)).toHaveLength(1);
  });

  it("has no reference and no estimate when no provider lists a price", () => {
    const pricing = buildModelPricing(model, "no-offers", { usage: null, mixes: MIXES });
    expect(pricing?.offerings).toEqual([]);
    expect(pricing?.reference).toBeNull();
    expect(pricing?.blend).toBeNull();
    expect(pricing?.community).toEqual({ status: "no-price" });
  });
});

describe("buildModelSpecs", () => {
  it("copies the models.dev facts with the retrieval time", () => {
    expect(buildModelSpecs(model, "opus")).toEqual({
      releaseDate: "2026-09-22",
      knowledgeCutoff: "2026-06",
      context: 1_000_000,
      maxOutput: 128_000,
      inputModalities: ["text", "image"],
      outputModalities: ["text"],
      openWeights: false,
      reasoning: true,
      toolCall: true,
      retrievedAt: "2026-10-21T01:00:00.000Z",
    });
    expect(buildModelSpecs(model, "bench-only")).toBeNull();
  });
});

describe("buildModelResearch", () => {
  it("credits only the sources the page shows", () => {
    const full = buildModelResearch(model, "opus", { usage: { agents: AGENTS }, mixes: MIXES });
    expect(full.sources.map((note) => note.id)).toEqual(["epoch", "tbench", "models.dev"]);

    const benchOnly = buildModelResearch(model, "bench-only", { usage: null, mixes: MIXES });
    expect(benchOnly.pricing).toBeNull();
    expect(benchOnly.specs).toBeNull();
    expect(benchOnly.sources.map((note) => note.id)).toEqual(["epoch"]);

    const specOnly = buildModelResearch(model, "spec-only", { usage: null, mixes: MIXES });
    expect(specOnly.benchmarks.rows).toEqual([]);
    expect(specOnly.sources.map((note) => note.id)).toEqual(["models.dev"]);
  });
});
