import { describe, expect, it } from "vitest";
import { createBenchmarkModel } from "@/lib/benchmarks/model";
import type { BenchmarkContent } from "@/lib/benchmarks/content";
import type { BenchmarkResult, ModelSpec, SourceSnapshot } from "@/lib/benchmarks/schema";
import type { ModelUsageRow } from "@/lib/explore/types";
import {
  axisLabel,
  benchmarkIndexSummary,
  benchmarkMetaDescription,
  benchmarkPageInfo,
  benchmarkPageSlugs,
  bestResultIndexes,
  buildBenchmarkIndex,
  buildModelContext,
  buildResultRows,
  buildTimeline,
  competitionRanks,
  dateAxis,
  dayNumber,
  describeConditions,
  firstSentence,
  formatStderr,
  mappingSummary,
  newestResultDate,
  niceStep,
  resultColumns,
  resultDate,
  scoreAxis,
  timelineGeometry,
  type TimelinePoint,
} from "@/lib/benchmarks/detail";

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

function spec(modelKey: string, releaseDate: string | null): ModelSpec {
  return {
    modelKey,
    name: modelKey,
    labProvider: null,
    releaseDate,
    knowledgeCutoff: null,
    context: null,
    maxOutput: null,
    inputModalities: ["text"],
    outputModalities: ["text"],
    openWeights: null,
    reasoning: null,
    toolCall: null,
    offerings: [],
  };
}

function content(): BenchmarkContent {
  const epoch: SourceSnapshot = {
    source: "epoch",
    retrievedAt: "2026-10-21T01:00:00.000Z",
    fetchUrls: ["https://epoch.ai/data/benchmark_data.zip"],
    dataDate: "2026-10-15",
    benchmarks: [
      {
        slug: "epoch-capabilities-index",
        sourceName: "Epoch Capabilities Index",
        meta: null,
        results: [
          mapped("claude-opus-5-5", "Claude Opus 5.5", 168.8, { name: "Claude Opus 5.5", ciLow: 165.3, ciHigh: 174, modelReleaseDate: "2026-09-22" }),
          mapped("gpt-6-astra", "GPT-6 Astra", 168.8, { name: "GPT-6 Astra", modelReleaseDate: "2026-09-03" }),
          result({ externalId: "Old Model", score: 120, modelReleaseDate: "2024-03-01" }),
        ],
      },
      {
        slug: "gpqa-diamond",
        sourceName: "GPQA diamond",
        meta: { randomBaseline: 0.25, ceiling: 0.97, releaseDate: "2023-11-20", supersededBy: null },
        results: [
          mapped("claude-opus-5-5", "opus_high", 0.9, { effort: "high", runDate: "2026-10-01", modelReleaseDate: "2026-09-22", stderr: 0.0119 }),
          mapped("claude-opus-5-5", "opus_max", 0.95, { effort: "max", runDate: "2026-10-02", modelReleaseDate: "2026-09-22", mapping: "normalized" }),
          mapped("gpt-6-astra", "astra_32k", 0.95, { thinkingBudget: "32K", runDate: "2026-10-03", modelReleaseDate: "2026-09-03", mapping: "manual", reviewed: true }),
          result({ externalId: "mystery_low", externalModel: "mystery", score: 0.5, effort: "low", runDate: "2025-01-02" }),
          result({ externalId: "mystery_high", externalModel: "mystery", score: 0.6, effort: "high", runDate: "2025-01-03" }),
        ],
      },
    ],
  };
  const tbench: SourceSnapshot = {
    source: "tbench",
    retrievedAt: "2026-10-21T02:00:00.000Z",
    fetchUrls: ["https://www.tbench.ai/leaderboard"],
    dataDate: "2026-10-06",
    benchmarks: [
      {
        slug: "terminal-bench-4",
        sourceName: "Terminal-Bench 4.0",
        meta: null,
        results: [
          mapped("claude-opus-5-5", "Claude Code / Opus 5.5 / max", 0.6485, {
            name: "Opus 5.5",
            agent: "Claude Code",
            effort: "max",
            ciLow: 0.6174,
            ciHigh: 0.6796,
            costUsd: 4718.65,
            costBasis: "per run",
            tokens: 8_034_082_467,
            publishedDate: "2026-10-06",
            modelReleaseDate: "2026-09-22",
            notes: "330 trials",
          }),
        ],
      },
    ],
  };
  const lmarena: SourceSnapshot = {
    source: "lmarena",
    retrievedAt: "2026-10-20T23:00:00.000Z",
    fetchUrls: ["https://huggingface.co/datasets/lmarena-ai/leaderboard-dataset"],
    dataDate: "2026-10-19",
    benchmarks: [
      {
        slug: "webdev-arena",
        sourceName: "webdev",
        meta: null,
        results: [
          mapped("claude-opus-5-5", "Claude Opus 5.5 (Max)", 1807.3, { name: "Claude Opus 5.5 (Max)", votes: 2925, publishedDate: "2026-10-19" }),
          mapped("gpt-6-astra", "GPT-6 Astra (Max)", 1773.8, { votes: 1, publishedDate: "2026-10-19" }),
          mapped("no-date-model", "No Date (High)", 1500, { publishedDate: "2026-10-19" }),
        ],
      },
      {
        slug: "agent-arena",
        sourceName: "agent",
        meta: null,
        results: [mapped("gpt-6-astra", "GPT-6 Astra", -0.05, { sessions: 11182, agent: "Arena Agent Mode", publishedDate: "2026-10-19" })],
      },
    ],
  };
  return {
    sources: new Map([
      ["epoch", epoch],
      ["tbench", tbench],
      ["lmarena", lmarena],
    ]),
    catalog: {
      source: "models.dev",
      retrievedAt: "2026-10-21T01:00:00.000Z",
      fetchUrls: ["https://models.dev/api.json"],
      providers: {},
      models: [spec("claude-opus-5-5", "2026-09-21"), spec("gpt-6-astra", "2026-09-04")],
    },
    report: { generatedAt: "2026-10-21T01:00:00.000Z", sources: [], methods: { exact: 0, normalized: 0, manual: 0, unreviewed: 0 }, unmapped: [] },
  };
}

function usage(key: string, activeUsers: number, name = key): ModelUsageRow {
  return {
    key,
    name,
    lab: null,
    labSlug: null,
    logoUrl: `/logo/${key}.svg`,
    rank: 0,
    activeUsers,
    previousActiveUsers: null,
    adoption: 0,
    tokens: activeUsers * 1000,
    estimatedCost: 0,
    isNew: false,
    topAgents: [],
  };
}

const model = createBenchmarkModel(content());
const gpqa = () => {
  const b = model.get("gpqa-diamond");
  if (!b) throw new Error("missing gpqa");
  return b;
};

describe("result dates", () => {
  it("prefers the run date, then the publish date, then the model release", () => {
    expect(resultDate(result({ externalId: "a", score: 1, runDate: "2026-01-02", publishedDate: "2026-01-03" }))).toEqual({ date: "2026-01-02", kind: "run" });
    expect(resultDate(result({ externalId: "a", score: 1, publishedDate: "2026-01-03", modelReleaseDate: "2026-01-01" }))).toEqual({ date: "2026-01-03", kind: "published" });
    expect(resultDate(result({ externalId: "a", score: 1, modelReleaseDate: "2026-01-01" }))).toEqual({ date: "2026-01-01", kind: "release" });
    expect(resultDate(result({ externalId: "a", score: 1 }))).toBeNull();
  });

  it("finds the newest date and says which kind it is", () => {
    const newest = newestResultDate([
      result({ externalId: "a", score: 1, modelReleaseDate: "2026-02-01" }),
      result({ externalId: "b", score: 1, runDate: "2026-02-01" }),
      result({ externalId: "c", score: 1, publishedDate: "2026-01-15" }),
    ]);
    expect(newest).toEqual({ date: "2026-02-01", kind: "run" });
    expect(newestResultDate([])).toBeNull();
  });
});

describe("benchmark index", () => {
  it("lists only benchmarks with results as pages", () => {
    const slugs = benchmarkPageSlugs(model);
    expect(slugs).toContain("terminal-bench-4");
    expect(slugs).toContain("gpqa-diamond");
    expect(slugs).not.toContain("swe-bench-pro-v2");
    // A benchmark whose snapshot has no entry has no results and no page.
    expect(slugs).not.toContain("hle");
  });

  it("groups by category with labels and keeps link-only benchmarks without a page", () => {
    const groups = buildBenchmarkIndex(model);
    expect(groups.map((g) => g.category)).toEqual(["overall", "agentic-coding", "web-development", "agents", "reasoning", "knowledge", "history"]);
    expect(groups[1].label).toBe("Agentic coding");
    const tb = groups[1].items.find((i) => i.slug === "terminal-bench-4");
    expect(tb).toMatchObject({
      href: "/benchmarks/terminal-bench-4",
      modelCount: 1,
      externalModelCount: 1,
      resultCount: 1,
      retrievedAt: "2026-10-21",
      lastUpdate: { date: "2026-10-06", kind: "published" },
      licenseName: "Terms not stated",
    });
    const pro = groups[1].items.find((i) => i.slug === "swe-bench-pro-v2");
    expect(pro?.href).toBeNull();
    expect(pro?.retrievedAt).toBeNull();
    expect(pro?.notes).toEqual(["Results not republished. Open the leaderboard on Scale AI's site."]);
    const history = groups.find((g) => g.category === "history")?.items[0];
    expect(history?.notes[0]).toMatch(/^History only/);
  });

  it("summarises pages, sources and retrieval days", () => {
    expect(benchmarkIndexSummary(model)).toEqual({
      withResults: 5,
      linkOnly: 1,
      sources: ["Epoch AI", "LMArena", "Terminal-Bench"],
      retrievedFrom: "2026-10-20",
      retrievedTo: "2026-10-21",
    });
  });
});

describe("benchmark page info", () => {
  it("carries the header facts and the snapshot source name", () => {
    const info = benchmarkPageInfo(model, gpqa());
    expect(info).toMatchObject({
      slug: "gpqa-diamond",
      categoryLabel: "Reasoning",
      unit: "percent",
      unitLabel: "% correct",
      sourceName: "Epoch AI",
      retrievedAt: "2026-10-21",
      dataDate: "2026-10-15",
      lastUpdate: { date: "2026-10-03", kind: "run" },
      resultCount: 5,
      modelCount: 2,
      externalModelCount: 4,
    });
  });

  it("writes a description from the first sentence of what it measures", () => {
    expect(firstSentence("One. Two.")).toBe("One.");
    expect(firstSentence("No full stop")).toBe("No full stop");
    expect(benchmarkMetaDescription(gpqa())).toBe(
      "Hard multiple-choice questions in biology, chemistry and physics, written by PhD-level experts. GPQA Diamond results for 2 models, with the source, date and conditions of each score, next to Tokscale usage.",
    );
  });
});

describe("model matching summary", () => {
  it("counts matched and unmatched results by method", () => {
    expect(mappingSummary(gpqa().results)).toEqual({
      results: 5,
      matched: 3,
      unmatched: 2,
      models: 2,
      unmatchedModels: 1,
      exact: 1,
      normalized: 1,
      manual: 1,
      reviewed: 1,
    });
  });
});

describe("usage context", () => {
  const ctx = buildModelContext(model, [usage("claude-opus-5-5", 165, "Claude Opus 5.5"), usage("idle", 0)], [usage("old-only", 3)]);

  it("joins active users from the last 30 days and skips zero", () => {
    expect(ctx.activeUsers?.get("claude-opus-5-5")).toBe(165);
    expect(ctx.activeUsers?.has("idle")).toBe(false);
  });

  it("links models with Tokscale usage, a spec or a matched result", () => {
    expect(ctx.href("gpt-6-astra")).toBe("/models/gpt-6-astra");
    expect(ctx.href("old-only")).toBe("/models/old-only");
    expect(ctx.href("no-date-model")).toBe("/models/no-date-model");
    expect(ctx.href("never-seen")).toBeNull();
  });

  it("names models from usage rows, else from the key", () => {
    expect(ctx.ref("claude-opus-5-5")).toEqual({ name: "Claude Opus 5.5", logoUrl: "/logo/claude-opus-5-5.svg" });
    expect(ctx.ref("gpt-6-astra").name).toBe("GPT-6 Astra");
  });

  it("has no usage map when usage failed to load", () => {
    const offline = buildModelContext(model, null, null);
    expect(offline.activeUsers).toBeNull();
    expect(offline.href("claude-opus-5-5")).toBe("/models/claude-opus-5-5");
    expect(offline.href("old-only")).toBeNull();
  });
});

describe("result rows", () => {
  const ctx = buildModelContext(model, [usage("claude-opus-5-5", 165, "Claude Opus 5.5")], []);

  it("ranks ties together", () => {
    expect(competitionRanks([0.95, 0.95, 0.9, 0.6, 0.6, 0.5])).toEqual([1, 1, 3, 4, 4, 6]);
    expect(competitionRanks([])).toEqual([]);
  });

  it("picks one best result per Tokscale model and per unmatched source model", () => {
    const results = gpqa().results;
    const best = [...bestResultIndexes(results)].map((i) => results[i].externalId).sort();
    expect(best).toEqual(["astra_32k", "mystery_high", "opus_max"]);
  });

  it("builds rows with both ranks, conditions, intervals and usage", () => {
    const rows = buildResultRows(gpqa(), ctx);
    expect(rows.map((r) => [r.sourceName, r.rank, r.bestRank])).toEqual([
      ["astra_32k", 1, 1],
      ["opus_max", 1, 1],
      ["opus_high", 3, null],
      ["mystery_high", 4, 3],
      ["mystery_low", 5, null],
    ]);
    const opusHigh = rows[2];
    expect(opusHigh).toMatchObject({
      modelKey: "claude-opus-5-5",
      modelName: "Claude Opus 5.5",
      href: "/models/claude-opus-5-5",
      scoreText: "90.0%",
      interval: { kind: "se", text: "\u00b11.2 pts" },
      conditions: "High effort",
      date: { date: "2026-10-01", kind: "run" },
      activeUsers: 165,
    });
    expect(rows[0]).toMatchObject({ conditions: "32K thinking budget", activeUsers: null, href: "/models/gpt-6-astra" });
    expect(rows[3]).toMatchObject({ modelKey: null, modelName: null, href: null, activeUsers: null, sourceName: "mystery_high" });
    expect(rows.map((r) => r.id)).toEqual(["0", "1", "2", "3", "4"]);
  });

  it("formats cost, tokens, intervals and samples for agent boards and arenas", () => {
    const tb = model.get("terminal-bench-4");
    const arena = model.get("webdev-arena");
    const agents = model.get("agent-arena");
    if (!tb || !arena || !agents) throw new Error("missing benchmarks");
    const [row] = buildResultRows(tb, ctx);
    expect(row).toMatchObject({
      sourceName: "Opus 5.5",
      scoreText: "64.8%",
      interval: { kind: "ci", text: "61.7\u201368.0%" },
      conditions: "Max effort \u00b7 Claude Code",
      note: "330 trials",
      cost: "$4,719 per run",
      tokens: 8_034_082_467,
      basis: "per run",
    });
    const arenaRows = buildResultRows(arena, ctx);
    expect(arenaRows.map((r) => r.sample)).toEqual(["2,925 votes", "1 vote", null]);
    expect(arenaRows[1].sourceName).toBe("GPT-6 Astra (Max)");
    expect(buildResultRows(agents, ctx)[0]).toMatchObject({ sample: "11,182 sessions", scoreText: "\u22120.05" });
    expect(resultColumns(buildResultRows(tb, ctx))).toEqual({ conditions: true, cost: true });
    expect(resultColumns(arenaRows)).toEqual({ conditions: false, cost: false });
  });

  it("describes conditions and standard errors in plain words", () => {
    expect(describeConditions({ effort: "none", thinkingBudget: null, agent: null })).toBe("No reasoning");
    expect(describeConditions({ effort: "xhigh", thinkingBudget: "32K", agent: "Codex" })).toBe("XHigh effort \u00b7 32K thinking budget \u00b7 Codex");
    expect(describeConditions({ effort: null, thinkingBudget: null, agent: null })).toBeNull();
    expect(formatStderr("percent", 0.0187)).toBe("\u00b11.9 pts");
    expect(formatStderr("index", 1.234)).toBe("\u00b11.2");
    expect(formatStderr("rating", 4.6)).toBe("\u00b15");
    expect(formatStderr("effect", 0.012)).toBe("\u00b10.01");
  });
});

describe("score vs release date", () => {
  const ctx = buildModelContext(model, [usage("claude-opus-5-5", 165)], []);

  it("uses the source's release date, else the models.dev date, and counts undated models", () => {
    const webdev = model.get("webdev-arena");
    if (!webdev) throw new Error("missing webdev");
    const timeline = buildTimeline(webdev, model.best("webdev-arena"), ctx);
    expect(timeline.points.map((p) => [p.key, p.releaseDate, p.releaseFrom])).toEqual([
      ["gpt-6-astra", "2026-09-04", "catalog"],
      ["claude-opus-5-5", "2026-09-21", "catalog"],
    ]);
    expect(timeline).toMatchObject({ models: 3, undated: 1, catalogDates: 2 });

    const g = buildTimeline(gpqa(), model.best("gpqa-diamond"), ctx);
    expect(g.points.map((p) => [p.key, p.releaseDate, p.releaseFrom, p.scoreText, p.activeUsers])).toEqual([
      ["gpt-6-astra", "2026-09-03", "source", "95.0%", null],
      ["claude-opus-5-5", "2026-09-22", "source", "95.0%", 165],
    ]);
    expect(g.catalogDates).toBe(0);
  });

  it("picks nice steps and labels in the unit", () => {
    expect(niceStep(0.86, 5)).toBe(0.2);
    expect(niceStep(81, 5)).toBe(20);
    expect(niceStep(736, 5)).toBe(200);
    expect(niceStep(0, 5)).toBe(1);
    expect(axisLabel("percent", 0.6, 0.2)).toBe("60%");
    expect(axisLabel("percent", 0.925, 0.025)).toBe("92.5%");
    expect(axisLabel("rating", 1400, 200)).toBe("1,400");
    expect(axisLabel("index", 150, 20)).toBe("150");
    expect(axisLabel("effect", -0.1, 0.1)).toBe("\u22120.1");
    expect(axisLabel("effect", 1e-17, 0.1)).toBe("0.0");
  });

  it("pads the score axis to whole steps and keeps percentages within 0-100%", () => {
    const pct = scoreAxis("percent", [0.093, 0.964]);
    expect([pct.min, pct.max]).toEqual([0, 1]);
    expect(pct.ticks.map((t) => t.label)).toEqual(["0%", "20%", "40%", "60%", "80%", "100%"]);
    const rating = scoreAxis("rating", [1071.5, 1807.3]);
    expect([rating.min, rating.max]).toEqual([1000, 1800 + 200]);
    const effect = scoreAxis("effect", [-0.157, 0.204]);
    expect(effect.ticks.map((t) => t.label)).toEqual(["\u22120.2", "\u22120.1", "0.0", "+0.1", "+0.2", "+0.3"].slice(0, effect.ticks.length));
    expect(effect.min).toBeLessThanOrEqual(-0.157);
    expect(effect.max).toBeGreaterThanOrEqual(0.204);
    const single = scoreAxis("index", [150]);
    expect(single.max).toBeGreaterThan(single.min);
  });

  it("puts date ticks on month starts, with years on January", () => {
    const short = dateAxis(dayNumber("2026-05-28"), dayNumber("2026-09-29"));
    // The axis is padded by a week on each side, so Oct 1 is on it too.
    expect(short.ticks.map((t) => t.label)).toEqual(["Jun 2026", "Jul", "Aug", "Sep", "Oct"]);
    const long = dateAxis(dayNumber("2024-02-26"), dayNumber("2026-10-15"));
    expect(long.ticks.map((t) => t.label)).toEqual(["Jul 2024", "2025", "Jul", "2026", "Jul"]);
    for (const t of long.ticks) expect(t.value).toBeGreaterThanOrEqual(long.min);
    expect(dayNumber("1970-01-02")).toBe(1);
  });

  it("places points inside the plot and draws a rising best-so-far line", () => {
    const points: TimelinePoint[] = [
      { key: "a", name: "A", href: null, score: 0.4, scoreText: "40.0%", conditions: null, releaseDate: "2025-01-01", releaseFrom: "source", activeUsers: null },
      { key: "b", name: "B", href: null, score: 0.3, scoreText: "30.0%", conditions: null, releaseDate: "2025-06-01", releaseFrom: "source", activeUsers: 2 },
      { key: "c", name: "C", href: null, score: 0.8, scoreText: "80.0%", conditions: null, releaseDate: "2026-01-01", releaseFrom: "source", activeUsers: 5 },
    ];
    const geometry = timelineGeometry(points, "percent");
    for (const p of geometry.positions) {
      expect(p.x).toBeGreaterThanOrEqual(0);
      expect(p.x).toBeLessThanOrEqual(1);
      expect(p.y).toBeGreaterThanOrEqual(0);
      expect(p.y).toBeLessThanOrEqual(1);
    }
    // Higher scores sit higher (smaller y); later releases sit further right.
    expect(geometry.positions[2].y).toBeLessThan(geometry.positions[0].y);
    expect(geometry.positions[2].x).toBeGreaterThan(geometry.positions[1].x);
    // Record setters are A then C: A, a step to C's x at A's height, C, then hold to the newest release.
    expect(geometry.frontier).toEqual([
      geometry.positions[0],
      { x: geometry.positions[2].x, y: geometry.positions[0].y },
      geometry.positions[2],
      { x: geometry.positions[2].x, y: geometry.positions[2].y },
    ]);
  });
});
