import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { ModelBenchmarks, periodPhrase, sourceHost } from "@/components/explore/ModelBenchmarks";
import { ModelNoUsageView } from "@/components/explore/ModelNoUsageView";
import { ModelPricing } from "@/components/explore/ModelPricing";
import { ModelSpecs } from "@/components/explore/ModelSpecs";
import type { BenchmarkContent } from "@/lib/benchmarks/content";
import { createBenchmarkModel } from "@/lib/benchmarks/model";
import {
  buildModelBenchmarks,
  buildModelPricing,
  buildModelResearch,
  buildModelSpecs,
  researchLabel,
  researchParts,
  type ModelSpecsView,
} from "@/lib/benchmarks/modelPage";
import type { BenchmarkResult, ModelSpec, SourceSnapshot } from "@/lib/benchmarks/schema";

function text(markup: string): string {
  return markup
    .replace(/<[^>]+>/g, " ")
    .replace(/&#x27;/g, "'")
    .replace(/&quot;/g, '"')
    .replace(/&amp;/g, "&")
    .replace(/\s+/g, " ");
}

function result(partial: Partial<BenchmarkResult> & Pick<BenchmarkResult, "externalId" | "score">): BenchmarkResult {
  return {
    externalModel: partial.externalId,
    name: null,
    modelKey: null,
    mapping: "exact",
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
    sourceUrl: "https://www.tbench.ai/leaderboard",
    notes: null,
    ...partial,
  };
}

const SPEC: ModelSpec = {
  modelKey: "opus",
  name: "Opus",
  labProvider: "anthropic",
  releaseDate: "2026-09-22",
  knowledgeCutoff: null,
  context: 1_000_000,
  maxOutput: null,
  inputModalities: ["text", "image"],
  outputModalities: [],
  openWeights: null,
  reasoning: true,
  toolCall: false,
  offerings: [
    { provider: "anthropic", id: "claude-opus", input: 4, output: 20, cacheRead: 0.2, cacheWrite: null, context: 1_000_000, maxOutput: null },
    ...["a", "b", "c", "d", "e"].map((p) => ({ provider: p, id: `${p}/opus`, input: 4, output: 20, cacheRead: 0.2, cacheWrite: 5, context: null, maxOutput: null })),
  ],
};

function content(): BenchmarkContent {
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
          result({ externalId: "cc/opus", score: 0.6485, modelKey: "opus", agent: "Claude Code", agentKey: "claude", effort: "max", publishedDate: "2026-10-06" }),
          result({ externalId: "codex/opus", score: 0.5, modelKey: "opus", agent: "Codex", agentKey: "codex", effort: "high", publishedDate: "2026-09-01" }),
          result({ externalId: "cc/sol", score: 0.7, modelKey: "sol", agent: "Codex", agentKey: "codex", effort: "max" }),
        ],
      },
    ],
  };
  return {
    sources: new Map([["tbench", tbench]]),
    catalog: {
      source: "models.dev",
      retrievedAt: "2026-10-21T01:00:00.000Z",
      fetchUrls: ["https://models.dev/api.json"],
      providers: { anthropic: { name: "Anthropic", url: null } },
      models: [SPEC],
    },
    report: { generatedAt: "2026-10-21T01:00:00.000Z", sources: [], methods: { exact: 0, normalized: 0, manual: 0, unreviewed: 0 }, unmapped: [] },
  };
}

const model = createBenchmarkModel(content());
const MIXES = {
  community: { input: 10, output: 10, cacheRead: 970, cacheWrite: 10, reasoning: 0 },
  byModel: { opus: { input: 5, output: 5, cacheRead: 980, cacheWrite: 10, reasoning: 0 } },
};

describe("model page helpers", () => {
  it("phrases the usage period", () => {
    expect(periodPhrase("7d")).toBe("in the last 7 days");
    expect(periodPhrase("30d")).toBe("in the last 30 days");
    expect(periodPhrase("all")).toBe("across all time");
  });

  it("names the site a source link opens", () => {
    expect(sourceHost("https://www.tbench.ai/leaderboard")).toBe("tbench.ai");
    expect(sourceHost("https://epoch.ai/benchmarks")).toBe("epoch.ai");
    expect(sourceHost("not a url")).toBe("Source");
  });
});

describe("ModelBenchmarks", () => {
  it("shows the best score with its rank, agent usage next to it, and the rest as not evaluated", () => {
    const view = buildModelBenchmarks(model, "opus", {
      agents: [{ key: "claude", name: "Claude Code", logoUrl: null, share: 0.9, activeUsers: 146 }],
    });
    const markup = text(renderToStaticMarkup(<ModelBenchmarks benchmarks={view} modelName="Opus" period="30d" />));
    expect(markup).toContain("Terminal-Bench 4.0");
    expect(markup).toContain("64.8%");
    expect(markup).toMatch(/#2 of 2/);
    expect(markup).toContain("Claude Code carried 90% of Opus tokens in the last 30 days (146 active users).");
    expect(markup).toContain("no Opus tokens went through Codex CLI in the last 30 days.");
    expect(markup).toContain("Show 1 more result with other run conditions");
    expect(markup).toContain("Not evaluated:");
    expect(markup).toContain("Epoch Capabilities Index");
    expect(markup).toContain("A missing result is not a score of zero.");
  });

  it("leaves out agent usage on pages without usage", () => {
    const view = buildModelBenchmarks(model, "opus", null);
    const markup = text(renderToStaticMarkup(<ModelBenchmarks benchmarks={view} modelName="Opus" period={null} />));
    expect(markup).not.toContain("Tokscale usage:");
    expect(markup).not.toContain("Usage does not explain the score.");
  });
});

describe("ModelPricing", () => {
  it("labels the effective price as an estimate and folds the long provider list", () => {
    const pricing = buildModelPricing(model, "opus", { usage: { agents: [] }, mixes: MIXES });
    const markup = text(
      renderToStaticMarkup(<ModelPricing pricing={pricing} modelName="Opus" period="30d" retrievedAt="2026-10-21T01:00:00.000Z" />),
    );
    expect(markup).toContain("Estimate at list prices");
    expect(markup).toContain("It is not what anyone paid.");
    expect(markup).toContain("Anthropic is the lab's own API");
    expect(markup).toContain("$8");
    expect(markup).toContain("98% cache reads");
    expect(markup).toContain("Show 2 more providers");
    expect(markup).toContain("No cache-write price is listed, so cache writes are priced as input.");
    expect(markup).toContain("$4 input per 1M tokens");
    expect(markup).toContain("Cache write not listed");
    expect(markup).toContain("Context not listed");
  });

  it("says the estimate is unavailable without usage data instead of showing a number", () => {
    const pricing = buildModelPricing(model, "opus", { usage: null, mixes: null });
    const markup = text(renderToStaticMarkup(<ModelPricing pricing={pricing} modelName="Opus" period="30d" retrievedAt={null} />));
    expect(markup).toContain("Unavailable: no Tokscale usage data");
    expect(markup).toContain("The estimate needs Tokscale usage data, which is not available here.");
    expect(markup).not.toContain("At this model");
  });

  it("explains a missing models.dev entry", () => {
    const markup = text(renderToStaticMarkup(<ModelPricing pricing={null} modelName="Sol" period="30d" retrievedAt={null} />));
    expect(markup).toContain("models.dev does not list Sol");
  });
});

describe("ModelSpecs", () => {
  it("marks values models.dev does not list instead of writing zero or no", () => {
    const specs = buildModelSpecs(model, "opus") as ModelSpecsView;
    const markup = text(renderToStaticMarkup(<ModelSpecs specs={specs} modelName="Opus" />));
    expect(markup).toContain("Context window 1M tokens");
    expect(markup).toContain("Max output \u2014 not listed");
    expect(markup).toContain("Output \u2014 not listed");
    expect(markup).toContain("Knowledge cutoff \u2014 not listed");
    expect(markup).toContain("Open weights \u2014 not listed");
    expect(markup).toContain("Tool calling No");
    expect(markup).toContain("Release date Sep 22, 2026");
    expect(markup).not.toMatch(/\b0 tokens\b/);
  });
});

describe("ModelNoUsageView", () => {
  it("says there is no usage yet and names only the sections that have data", () => {
    const research = buildModelResearch(model, "sol", { usage: null, mixes: MIXES });
    const markup = text(
      renderToStaticMarkup(
        <ModelNoUsageView
          model={{ key: "sol", name: "Sol", lab: null, labSlug: null, logoUrl: null }}
          research={research}
          period="30d"
          shows={researchLabel(researchParts(model, "sol")).phrase}
        />,
      ),
    );
    expect(markup).toContain("No public Tokscale usage yet");
    expect(markup).toContain("The benchmark results below come from public sources.");
    expect(markup).toContain("70.0%");
    expect(markup).toContain("models.dev does not list Sol");
    expect(markup).not.toContain("Tokscale usage:");
    expect(markup).toContain('Key sol');
  });
});
