import { describe, expect, it } from "vitest";
import { aliasRulesForWrite, createSyncContext, mapExternalModel } from "@/lib/benchmarks/sync/context";
import { buildEpochSnapshot, parseEciScores, parseEpochBenchmark, parseModelGroups } from "@/lib/benchmarks/sync/epoch";
import { lmarenaFilterUrl, parseArenaRows } from "@/lib/benchmarks/sync/lmarena";
import { buildCatalogSnapshot, catalogKeyFor, type ModelsDevApi } from "@/lib/benchmarks/sync/modelsdev";
import { buildReport, formatSnapshotJson } from "@/lib/benchmarks/sync/output";
import { decodeFlightPayload, extractTbenchLeaderboard, parseTbenchRows, sliceBalancedJson } from "@/lib/benchmarks/sync/tbench";
import { sourceSnapshotSchema, catalogSnapshotSchema } from "@/lib/benchmarks/schema";

const KNOWN = new Set(["claude-opus-5-5", "gpt-6.1-sol", "kimi-k3", "gemini-4-argon", "glm-5.3"]);

function flightHtml(payload: string): string {
  // Next.js splits the flight payload across escaped string chunks.
  const half = Math.floor(payload.length / 2);
  return [payload.slice(0, half), payload.slice(half)]
    .map((chunk) => `<script>self.__next_f.push([1,${JSON.stringify(chunk)}])</script>`)
    .join("");
}

const TBENCH_ROWS = [
  {
    rank: 1,
    status: "display",
    created_at: "2026-10-06T06:44:27Z",
    n_trials: 330,
    metadata: {
      date: "2026-09-22",
      reasoning_effort: "max",
      agent_display: { label: "Claude Code" },
      model_display: { url: "https://www.anthropic.com/claude-opus-5-5", label: "Opus 5.5" },
      model_org: { label: "Anthropic" },
    },
    metrics: { accuracy: 64.85, accuracy_ci95_half_width: 3.11, total_cost_usd: 4718.65, total_tokens: 8034082467 },
  },
  {
    rank: 32,
    status: "display",
    created_at: "2026-10-06T06:44:27Z",
    metadata: {
      reasoning_effort: "xhigh",
      agent_display: { label: "Muse Code" },
      model_display: { url: "https://ai.meta.com/tools/muse/", label: "Muse Spark 1.3" },
      model_org: { label: "Meta" },
    },
    metrics: { accuracy: 14.55, total_cost_usd: 0 },
  },
  { rank: 40, status: "hidden", metadata: { model_display: { label: "Hidden" } }, metrics: { accuracy: 1 } },
];

describe("Terminal-Bench page parsing", () => {
  it("decodes the flight payload and slices balanced JSON with brackets inside strings", () => {
    const text = 'x"rows":[{"a":"]}"},{"b":[1,2]}] tail';
    const start = text.indexOf("[");
    expect(sliceBalancedJson(text, start)).toBe('[{"a":"]}"},{"b":[1,2]}]');
    expect(decodeFlightPayload(flightHtml("hello world"))).toBe("hello world");
  });

  it("finds the Terminal-Bench 4.0 leaderboard rows in the page", () => {
    const payload = `0:{"other":1}\n5:{"title":"Terminal-Bench 4.0","updated_at":"2026-10-06T23:20:25Z","rows":${JSON.stringify(TBENCH_ROWS)}}`;
    const board = extractTbenchLeaderboard(flightHtml(payload));
    expect(board.rows).toHaveLength(3);
    expect(board.updatedAt).toBe("2026-10-06T23:20:25Z");
    expect(() => extractTbenchLeaderboard(flightHtml('{"title":"Other"}'))).toThrow(/not found/);
  });

  it("keeps result rows only, as fractions, with conditions, and never reports $0 as a cost", () => {
    const ctx = createSyncContext(KNOWN, []);
    const rows = parseTbenchRows(TBENCH_ROWS, ctx);
    expect(rows).toHaveLength(2);
    expect(rows[0]).toMatchObject({
      modelKey: "claude-opus-5-5",
      mapping: "normalized",
      effort: "max",
      agent: "Claude Code",
      agentKey: "claude",
      score: 0.6485,
      ciLow: 0.6174,
      ciHigh: 0.6796,
      costUsd: 4718.65,
      costBasis: "per run",
      publishedDate: "2026-10-06",
      modelReleaseDate: "2026-09-22",
      sourceUrl: "https://www.tbench.ai/leaderboard",
    });
    expect(rows[1]).toMatchObject({ modelKey: null, agentKey: "muse", costUsd: null, costBasis: null, ciLow: null });
    expect([...ctx.unmapped.values()].map((u) => u.externalModel)).toEqual(["Muse Spark 1.3"]);
  });
});

describe("LMArena parsing", () => {
  it("builds a datasets-server filter URL for the overall category", () => {
    const url = new URL(lmarenaFilterUrl("webdev", 100));
    expect(url.searchParams.get("config")).toBe("webdev");
    expect(url.searchParams.get("where")).toBe(`"category"='overall'`);
    expect(url.searchParams.get("offset")).toBe("100");
  });

  it("maps names, splits effort and harness, and skips other categories", () => {
    const ctx = createSyncContext(KNOWN, []);
    const rows = parseArenaRows(
      "webdev-arena",
      [
        { model_name: "Claude Opus 5.5 (Max)", organization: "anthropic", rating: 1807.3, rating_lower: 1793.4, rating_upper: 1821.1, vote_count: 2925, rank: 1, category: "overall", leaderboard_publish_date: "2026-10-19" },
        { model_name: "GPT-6.1 Sol (xHigh, Codex Harness)", organization: "openai", rating: 1700, rank: 9, category: "overall", leaderboard_publish_date: "2026-10-19" },
        { model_name: "Claude Opus 5.5 (Max)", rating: 1500, rank: 1, category: "coding", leaderboard_publish_date: "2026-10-19" },
      ],
      ctx,
    );
    expect(rows).toHaveLength(2);
    expect(rows[0]).toMatchObject({ modelKey: "claude-opus-5-5", effort: "max", votes: 2925, ciLow: 1793.4, publishedDate: "2026-10-19" });
    expect(rows[1]).toMatchObject({ modelKey: "gpt-6.1-sol", effort: "xhigh", agent: "Codex", agentKey: "codex" });
  });

  it("reads Agent Arena effect scores and session counts", () => {
    const ctx = createSyncContext(KNOWN, []);
    const [row] = parseArenaRows(
      "agent-arena",
      [{ model_name: "Kimi K3 (High)", score: 0.2, score_ci_lower: 0.18, score_ci_upper: 0.22, session_count: 11182, rank: 1, category: "overall" }],
      ctx,
    );
    expect(row).toMatchObject({ modelKey: "kimi-k3", score: 0.2, sessions: 11182, agent: "Arena Agent Mode" });
  });
});

describe("Epoch parsing", () => {
  const groupsCsv = "model_version,model_group\nclaude-opus-5-5_max,Claude Opus 5.5\nclaude-opus-5-5_high,Claude Opus 5.5\nfireworks/kimi-k3,Kimi K3\n";

  it("groups model versions by model with conditions removed", () => {
    expect(parseModelGroups(groupsCsv).get("Claude Opus 5.5")).toEqual(["claude-opus-5-5"]);
  });

  it("maps ECI rows through the model metadata ids", () => {
    const ctx = createSyncContext(KNOWN, []);
    const eci = "Model,Display name,eci,eci_ci_low,eci_ci_high,date,Organization\nClaude Opus 5.5,Claude Opus 5.5,168.83,165.25,173.95,2026-09-22,Anthropic\nUnknown Model,Unknown Model,120,,,2026-01-01,Lab\n";
    const rows = parseEciScores(eci, parseModelGroups(groupsCsv), ctx);
    expect(rows[0]).toMatchObject({ modelKey: "claude-opus-5-5", mapping: "exact", score: 168.83, ciLow: 165.25, modelReleaseDate: "2026-09-22" });
    expect(rows[1].modelKey).toBeNull();
  });

  it("reads effort from the version suffix or the effort column and a 95% half-width", () => {
    const ctx = createSyncContext(KNOWN, []);
    const csv =
      "Model version,Pass@1,Harness,Reasoning effort,95% CI half-width,Mean cost (USD),Release date,Organization,Name,Source\n" +
      "gpt-6.1-sol_max,0.72,mini-swe-agent,max,0.02,8.38,2026-09-29,OpenAI,gpt-6-1-sol (max),https://deepswe.datacurve.ai/\n";
    const [row] = parseEpochBenchmark("deepswe", csv, ctx);
    expect(row).toMatchObject({ modelKey: "gpt-6.1-sol", effort: "max", agent: "mini-swe-agent", agentKey: null, ciLow: 0.7, ciHigh: 0.74, costUsd: 8.38, costBasis: "per task" });
    expect(row.sourceUrl).toBe("https://deepswe.datacurve.ai/");
  });

  it("fails loudly when the archive is missing a file", () => {
    expect(() => buildEpochSnapshot({}, createSyncContext(KNOWN, []), "2026-10-21T00:00:00.000Z")).toThrow(/missing/);
  });
});

describe("models.dev catalog", () => {
  const api: ModelsDevApi = {
    anthropic: {
      name: "Anthropic",
      doc: "https://docs.anthropic.com",
      models: {
        "claude-opus-5-5": {
          id: "claude-opus-5-5",
          name: "Claude Opus 5.5",
          release_date: "2026-09-22",
          knowledge: "2026-06",
          modalities: { input: ["text", "image"], output: ["text"] },
          open_weights: false,
          limit: { context: 1_000_000, output: 128_000 },
          cost: { input: 4, output: 20, cache_read: 0.2, cache_write: 5 },
        },
      },
    },
    openrouter: {
      name: "OpenRouter",
      models: {
        "anthropic/claude-opus-5.5": { id: "anthropic/claude-opus-5.5", canonical_model_id: "anthropic/claude-opus-5-5", modalities: { input: ["text"], output: ["text"] }, cost: { input: 4, output: 20 } },
        "anthropic/claude-opus-5.5-fast": { id: "anthropic/claude-opus-5.5-fast", canonical_model_id: "anthropic/claude-opus-5-5", modalities: { input: ["text"], output: ["text"] }, cost: { input: 8, output: 40 } },
        "some/model": { id: "some/model", modalities: { input: ["text"], output: ["text"] }, cost: { input: 1, output: 1 } },
      },
    },
  };

  it("uses canonical ids, trusts first-party ids, and skips third-party ids without one", () => {
    expect(catalogKeyFor("openrouter", api.openrouter.models!["anthropic/claude-opus-5.5"])).toBe("claude-opus-5-5");
    expect(catalogKeyFor("anthropic", api.anthropic.models!["claude-opus-5-5"])).toBe("claude-opus-5-5");
    expect(catalogKeyFor("openrouter", api.openrouter.models!["some/model"])).toBeNull();
  });

  it("puts the lab's own list price first and keeps one offering per provider", () => {
    const snapshot = catalogSnapshotSchema.parse(buildCatalogSnapshot(api, new Set(["claude-opus-5-5"]), "2026-10-21T00:00:00.000Z"));
    const [model] = snapshot.models;
    expect(model).toMatchObject({ modelKey: "claude-opus-5-5", labProvider: "anthropic", context: 1_000_000, knowledgeCutoff: "2026-06", openWeights: false });
    expect(model.offerings.map((o) => [o.provider, o.id])).toEqual([
      ["anthropic", "claude-opus-5-5"],
      ["openrouter", "anthropic/claude-opus-5.5"],
    ]);
    expect(snapshot.providers.anthropic).toEqual({ name: "Anthropic", url: "https://docs.anthropic.com" });
  });
});

describe("sync bookkeeping", () => {
  it("records mapped aliases, keeps manual rules, and reports unmapped models", () => {
    const manual = { source: "lmarena" as const, externalId: "Other", modelKey: "kimi-k3", method: "manual" as const, reviewed: true };
    const ctx = createSyncContext(KNOWN, [manual]);
    mapExternalModel(ctx, "epoch", { externalId: "glm-5.3", ids: ["glm-5.3"] }, { benchmark: "gpqa-diamond", name: null, date: "2026-08-14" });
    mapExternalModel(ctx, "epoch", { externalId: "mystery-1", ids: ["mystery-1"] }, { benchmark: "gpqa-diamond", name: null, date: "2026-08-14" });
    mapExternalModel(ctx, "epoch", { externalId: "mystery-1", ids: ["mystery-1"] }, { benchmark: "hle", name: null, date: "2026-09-01" });
    const rules = aliasRulesForWrite(ctx);
    expect(rules.map((r) => `${r.source}:${r.externalId}:${r.method}`)).toEqual(["epoch:glm-5.3:exact", "lmarena:Other:manual"]);
    const unmapped = ctx.unmapped.get("epoch\u0000mystery-1");
    expect([...(unmapped?.benchmarks ?? [])].sort()).toEqual(["gpqa-diamond", "hle"]);
    expect(unmapped?.newestDate).toBe("2026-09-01");

    const report = buildReport(ctx, [], null, [{ source: "epoch", ok: true, error: null }], rules, "2026-10-21T00:00:00.000Z");
    expect(report.methods).toEqual({ exact: 1, normalized: 0, manual: 1, unreviewed: 1 });
    expect(report.unmapped.map((u) => u.externalModel)).toEqual(["mystery-1"]);
  });

  it("formats snapshots with one flat array element per line", () => {
    const text = formatSnapshotJson({ a: 1, rows: [{ x: 1, y: [1, 2] }, { x: 2, y: [] }], nested: { z: null } });
    expect(text).toBe('{\n  "a": 1,\n  "rows": [\n    {"x":1,"y":[1,2]},\n    {"x":2,"y":[]}\n  ],\n  "nested": {\n    "z": null\n  }\n}\n');
    expect(JSON.parse(text)).toEqual({ a: 1, rows: [{ x: 1, y: [1, 2] }, { x: 2, y: [] }], nested: { z: null } });
  });

  it("validates a parsed snapshot with the schema", () => {
    const ctx = createSyncContext(KNOWN, []);
    const results = parseTbenchRows(TBENCH_ROWS, ctx);
    expect(() =>
      sourceSnapshotSchema.parse({
        source: "tbench",
        retrievedAt: "2026-10-21T00:00:00.000Z",
        fetchUrls: ["https://www.tbench.ai/leaderboard"],
        dataDate: "2026-10-06",
        benchmarks: [{ slug: "terminal-bench-4", sourceName: "Terminal-Bench 4.0", meta: null, results }],
      }),
    ).not.toThrow();
  });
});
