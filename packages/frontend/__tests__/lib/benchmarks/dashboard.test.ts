import { describe, expect, it } from "vitest";
import {
  columnGroupsParam,
  dashboardQuery,
  DEFAULT_COLUMN_GROUPS,
  effectiveSortKey,
  visibleSortKeys,
  groupForSortKey,
  parseColumnGroups,
  parseSortKey,
  sortAscending,
  toggleColumnGroup,
} from "@/lib/benchmarks/columns";
import { buildDashboardRows, selectDashboardKeys, sortDashboardRows, type DashboardRow } from "@/lib/benchmarks/dashboard";
import { conditionTag, formatContext, formatInterval, formatIsoDay, formatPricePerMillion, formatScore, formatUsd } from "@/lib/benchmarks/format";
import { createBenchmarkModel, saturationNote, staleNote, compareResults } from "@/lib/benchmarks/model";
import { getBenchmarkDefinition } from "@/lib/benchmarks/definitions";
import type { BenchmarkContent } from "@/lib/benchmarks/content";
import type { BenchmarkResult, SourceSnapshot } from "@/lib/benchmarks/schema";
import type { ModelUsageRow } from "@/lib/explore/types";

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
        results: [mapped("opus", "Opus", 168.8), mapped("sol", "Sol", 168.4), mapped("argon", "Argon", 166.5), result({ externalId: "Unmatched", score: 170 })],
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
        results: [mapped("opus", "Claude Code / Opus", 0.6485, { agent: "Claude Code", effort: "max", publishedDate: "2026-10-06", ciLow: 0.6174, ciHigh: 0.6796 })],
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
      providers: { anthropic: { name: "Anthropic", url: null } },
      models: [
        {
          modelKey: "opus",
          name: "Opus",
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
          offerings: [{ provider: "anthropic", id: "opus", input: 4, output: 20, cacheRead: 0.2, cacheWrite: 5, context: 1_000_000, maxOutput: 128_000 }],
        },
      ],
    },
    report: { generatedAt: "2026-10-21T01:00:00.000Z", sources: [], methods: { exact: 0, normalized: 0, manual: 0, unreviewed: 0 }, unmapped: [] },
  };
}

function usage(key: string, activeUsers: number): ModelUsageRow {
  return {
    key,
    name: key,
    lab: null,
    labSlug: null,
    logoUrl: null,
    rank: 0,
    activeUsers,
    previousActiveUsers: null,
    adoption: 0,
    tokens: activeUsers * 1000,
    estimatedCost: 0,
    isNew: false,
    topAgents: [{ key: "claude", name: "Claude Code", logoUrl: null, share: 0.9 }],
  };
}

describe("column groups in the URL", () => {
  it("parses and orders the groups; nothing valid means the overview", () => {
    expect(parseColumnGroups("coding,usage")).toEqual(["usage", "coding"]);
    expect(parseColumnGroups("nope")).toEqual([]);
    expect(parseColumnGroups(undefined)).toEqual([...DEFAULT_COLUMN_GROUPS]);
    expect(parseColumnGroups(["price,specs", "usage"])).toEqual(["price", "specs"]);
  });

  it("serializes only non-default groups", () => {
    expect(columnGroupsParam(DEFAULT_COLUMN_GROUPS)).toBeNull();
    expect(columnGroupsParam([])).toBeNull();
    expect(columnGroupsParam(["specs", "usage"])).toBe("usage,specs");
  });

  it("toggles groups and returns to the overview after the last one", () => {
    expect(toggleColumnGroup(["usage", "coding"], "price")).toEqual(["usage", "coding", "price"]);
    expect(toggleColumnGroup(["usage", "coding"], "usage")).toEqual(["coding"]);
    expect(toggleColumnGroup(["coding"], "coding")).toEqual([]);
  });

  it("keeps a sort only while its column is visible", () => {
    expect(visibleSortKeys([])).toEqual(["users", "epoch-capabilities-index", "terminal-bench-4", "webdev-arena", "price-community"]);
    expect(effectiveSortKey("terminal-bench-4", [])).toBe("terminal-bench-4");
    expect(effectiveSortKey("gpqa-diamond", [])).toBe("users");
    expect(effectiveSortKey("users", ["price"])).toBe("price-community");
    expect(effectiveSortKey("context", ["specs", "usage"])).toBe("context");
  });

  it("validates sort keys and knows their direction and group", () => {
    expect(parseSortKey("terminal-bench-4")).toBe("terminal-bench-4");
    expect(parseSortKey("drop table")).toBe("users");
    expect(sortAscending("price-community")).toBe(true);
    expect(sortAscending("epoch-capabilities-index")).toBe(false);
    expect(groupForSortKey("webdev-arena")).toBe("webdev");
    expect(groupForSortKey("epoch-capabilities-index")).toBe("capability");
    expect(groupForSortKey("context")).toBe("specs");
  });

  it("builds the shortest query string", () => {
    expect(dashboardQuery({ groups: DEFAULT_COLUMN_GROUPS, sort: "users", period: "30d", defaultPeriod: "30d" })).toBe("");
    expect(dashboardQuery({ groups: ["price"], sort: "price-community", period: "7d", defaultPeriod: "30d" })).toBe("?period=7d&cols=price&sort=price-community");
  });
});

describe("benchmark model", () => {
  const model = createBenchmarkModel(content());

  it("keeps the best result per model and ranks among matched models", () => {
    const entries = model.forModel("opus");
    const gpqa = entries.find((e) => e.benchmark.definition.slug === "gpqa-diamond");
    expect(gpqa?.best.externalId).toBe("opus_max");
    expect(gpqa?.results.map((r) => r.externalId)).toEqual(["opus_max", "opus_high"]);
    expect(gpqa?.rank).toBe(2);
    expect(gpqa?.rankedModels).toBe(2);
    expect(model.get("epoch-capabilities-index")?.modelCount).toBe(3);
    expect(model.get("epoch-capabilities-index")?.externalModelCount).toBe(4);
  });

  it("notes saturation from Epoch's ceiling and staleness from dates", () => {
    expect(model.get("gpqa-diamond")?.saturationNote).toMatch(/Near its ceiling/);
    expect(saturationNote(getBenchmarkDefinition("gpqa-diamond")!, { randomBaseline: 0.25, ceiling: 0.97, releaseDate: null, supersededBy: null }, 0.8)).toBeNull();
    expect(staleNote("2026-02-26", "2026-10-21")).toMatch(/237 days/);
    expect(staleNote("2026-10-01", "2026-10-21")).toBeNull();
  });

  it("returns specs, the reference offering and link-only benchmarks without results", () => {
    expect(model.spec("opus")?.context).toBe(1_000_000);
    expect(model.referenceOffering("opus")?.provider).toBe("anthropic");
    expect(model.referenceOffering("sol")).toBeNull();
    expect(model.get("swe-bench-pro-v2")?.results).toEqual([]);
    expect(model.modelKeys()).toEqual(new Set(["opus", "sol", "argon"]));
  });

  it("orders results by score, then newest date", () => {
    const a = result({ externalId: "a", score: 0.5, runDate: "2026-01-01" });
    const b = result({ externalId: "b", score: 0.5, runDate: "2026-02-01" });
    expect([a, b].sort(compareResults).map((r) => r.externalId)).toEqual(["b", "a"]);
  });
});

describe("dashboard rows", () => {
  const model = createBenchmarkModel(content());

  it("selects models with enough users, then the top models by ECI", () => {
    const keys = selectDashboardKeys([usage("sol", 3), usage("opus", 165), usage("minor", 6)], model.best("epoch-capabilities-index"), 5, 2);
    expect(keys).toEqual(["opus", "minor", "sol"]);
  });

  it("joins usage, best scores with conditions, prices and specs", () => {
    const mixes = {
      community: { input: 1, output: 1, cacheRead: 96, cacheWrite: 2, reasoning: 0 },
      byModel: { opus: { input: 0.5, output: 0.3, cacheRead: 98.2, cacheWrite: 1, reasoning: 0 } },
    };
    const rows = buildDashboardRows(model, [usage("opus", 165)], mixes, { minUsers: 5, topEci: 3 });
    expect(rows.map((r) => r.key)).toEqual(["opus", "sol", "argon"]);
    const [opus, sol] = rows;
    expect(opus.scores["terminal-bench-4"]).toMatchObject({ text: "64.8%", tag: "Max \u00b7 Claude Code", dateKind: "published", interval: "61.7\u201368.0%" });
    expect(opus.scores["gpqa-diamond"]).toMatchObject({ text: "95.0%", tag: "Max", resultCount: 2, dateKind: "run" });
    expect(opus.scores.hle).toBeNull();
    expect(opus.price?.own?.perMillion).toBeCloseTo(0.3264, 6);
    expect(opus.price?.community?.perMillion).toBeCloseTo(0.01 * 4 + 0.01 * 20 + 0.96 * 0.2 + 0.02 * 5, 6);
    expect(opus.spec?.openWeights).toBe(false);
    expect(opus.usage?.topAgent?.key).toBe("claude");
    expect(sol.usage).toBeNull();
    expect(sol.price).toBeNull();
    expect(sol.name).toBe("Sol");
  });

  it("shows no effective price without usage data", () => {
    const [opus] = buildDashboardRows(model, [usage("opus", 165)], null, { topEci: 0 });
    expect(opus.price).toMatchObject({ input: 4, community: null, own: null });
  });

  it("sorts with missing values last in both directions", () => {
    const rows = buildDashboardRows(model, [usage("opus", 165)], null, { topEci: 3 });
    const byTb = sortDashboardRows(rows, "terminal-bench-4", false).map((r: DashboardRow) => r.key);
    expect(byTb[0]).toBe("opus");
    const byPrice = sortDashboardRows(rows, "price-input", true).map((r) => r.key);
    expect(byPrice).toEqual(["opus", "sol", "argon"]);
    expect(sortDashboardRows(rows, "epoch-capabilities-index", false).map((r) => r.key)).toEqual(["opus", "sol", "argon"]);
  });
});

describe("benchmark formatting", () => {
  it("formats each unit", () => {
    expect(formatScore("percent", 0.6485)).toBe("64.8%");
    expect(formatScore("index", 168.83)).toBe("168.8");
    expect(formatScore("rating", 1807.31)).toBe("1,807");
    expect(formatScore("effect", 0.2)).toBe("+0.20");
    expect(formatScore("effect", -0.05)).toBe("\u22120.05");
    expect(formatInterval("percent", null, 0.5)).toBeNull();
    expect(formatInterval("index", 165.26, 173.96)).toBe("165.3\u2013174.0");
  });

  it("formats conditions, dates, prices and context windows", () => {
    expect(conditionTag("xhigh", "Codex")).toBe("XHigh \u00b7 Codex");
    expect(conditionTag(null, null, "32K")).toBe("32K budget");
    expect(conditionTag(null, null)).toBeNull();
    expect(formatIsoDay("2026-10-06")).toBe("Oct 6, 2026");
    expect(formatIsoDay("2026-06")).toBe("Jun 2026");
    expect(formatPricePerMillion(0.3264)).toBe("$0.33");
    expect(formatPricePerMillion(4)).toBe("$4");
    expect(formatPricePerMillion(12.5)).toBe("$12.50");
    expect(formatPricePerMillion(0.004)).toBe("$0.004");
    expect(formatPricePerMillion(null)).toBe("\u2014");
    expect(formatUsd(4718.65)).toBe("$4,719");
    expect(formatUsd(17.28)).toBe("$17.28");
    expect(formatContext(1_000_000)).toBe("1M");
    expect(formatContext(1_050_000)).toBe("1.05M");
    expect(formatContext(262_144)).toBe("262K");
    expect(formatContext(null)).toBe("\u2014");
  });
});
