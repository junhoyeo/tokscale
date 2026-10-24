import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { BenchmarkDetailView } from "@/components/explore/BenchmarkDetailView";
import { BenchmarkIndex } from "@/components/explore/BenchmarkIndex";
import { ScoreTimeline } from "@/components/explore/ScoreTimeline";
import { sourceNotes } from "@/lib/benchmarks/attribution";
import type { BenchmarkContent } from "@/lib/benchmarks/content";
import {
  benchmarkIndexSummary,
  benchmarkPageInfo,
  buildBenchmarkIndex,
  buildModelContext,
  buildResultRows,
  buildTimeline,
  mappingSummary,
  resultColumns,
} from "@/lib/benchmarks/detail";
import { createBenchmarkModel } from "@/lib/benchmarks/model";
import type { BenchmarkResult, SourceSnapshot } from "@/lib/benchmarks/schema";
import type { ModelUsageRow } from "@/lib/explore/types";

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
    sourceUrl: "https://epoch.ai/benchmarks/gpqa-diamond",
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
        slug: "gpqa-diamond",
        sourceName: "GPQA diamond",
        meta: null,
        results: [
          mapped("opus", "opus_max", 0.95, { effort: "max", runDate: "2026-10-02", modelReleaseDate: "2026-09-22", stderr: 0.014 }),
          mapped("opus", "opus_high", 0.9, { effort: "high", runDate: "2026-10-01", modelReleaseDate: "2026-09-22" }),
          mapped("sol", "sol_max", 0.96, { effort: "max", runDate: "2026-10-03", modelReleaseDate: "2026-10-01" }),
          mapped("old", "old_model", 0.6, { runDate: "2025-01-03", modelReleaseDate: "2024-12-01" }),
          result({ externalId: "mystery-model", score: 0.7, runDate: "2026-02-01" }),
        ],
      },
    ],
  };
  return {
    sources: new Map([["epoch", epoch]]),
    catalog: {
      source: "models.dev",
      retrievedAt: "2026-10-21T01:00:00.000Z",
      fetchUrls: ["https://models.dev/api.json"],
      providers: {},
      models: [],
    },
    report: { generatedAt: "2026-10-21T01:00:00.000Z", sources: [], methods: { exact: 0, normalized: 0, manual: 0, unreviewed: 0 }, unmapped: [] },
  };
}

function usage(key: string, activeUsers: number): ModelUsageRow {
  return {
    key,
    name: key.toUpperCase(),
    lab: null,
    labSlug: null,
    logoUrl: null,
    rank: 0,
    activeUsers,
    previousActiveUsers: null,
    adoption: 0,
    tokens: 1000,
    estimatedCost: 0,
    isNew: false,
    topAgents: [],
  };
}

const model = createBenchmarkModel(content());
const gpqa = model.get("gpqa-diamond");
if (!gpqa) throw new Error("missing benchmark");

function renderDetail(recent: ModelUsageRow[] | null) {
  const ctx = buildModelContext(model, recent, recent);
  const rows = buildResultRows(gpqa!, ctx);
  return text(
    renderToStaticMarkup(
      <BenchmarkDetailView
        info={benchmarkPageInfo(model, gpqa!)}
        rows={rows}
        columns={resultColumns(rows)}
        timeline={buildTimeline(gpqa!, model.best("gpqa-diamond"), ctx)}
        mapping={mappingSummary(gpqa!.results)}
        usageWindow={recent ? { start: "2026-09-20", end: "2026-10-19" } : null}
        sources={sourceNotes(model, { slugs: ["gpqa-diamond"] })}
      />,
    ),
  );
}

describe("BenchmarkDetailView", () => {
  it("lists one row per model with conditions, usage and unmatched entries", () => {
    const markup = renderDetail([usage("opus", 165), usage("sol", 141)]);
    expect(markup).toContain("GPQA Diamond");
    expect(markup).toContain("Best per model 4");
    expect(markup).toContain("All results 5");
    expect(markup).toContain("Showing 4 of 4 models");
    expect(markup).toContain("Max effort");
    expect(markup).toContain("Standard error \u00b11.4 pts");
    expect(markup).toContain("165 active users");
    expect(markup).toContain("mystery-model Not matched to a Tokscale model");
    expect(markup).toContain("No Tokscale usage in the last 30 days");
    expect(markup).toContain("It is shown next to the score and does not explain it.");
    expect(markup).toContain("4 of 5 results are matched to 3 Tokscale models: 4 by exact id.");
    expect(markup).toContain("never change which model a result belongs to");
  });

  it("hides the usage column when usage could not be loaded", () => {
    const markup = renderDetail(null);
    expect(markup).toContain("Tokscale usage could not be loaded, so the usage column is hidden.");
    expect(markup).not.toContain("Tokscale usage (30 days)");
  });
});

describe("ScoreTimeline", () => {
  const ctx = buildModelContext(model, [usage("opus", 165)], [usage("opus", 165)]);
  const timeline = buildTimeline(gpqa, model.best("gpqa-diamond"), ctx);

  it("draws dated models and labels missing usage in the data table", () => {
    const markup = text(
      renderToStaticMarkup(
        <ScoreTimeline timeline={timeline} unit="percent" unitLabel="% correct" benchmarkName="GPQA Diamond" sourceName="Epoch AI" usageAvailable />,
      ),
    );
    expect(markup).toContain("3 models, released Dec 1, 2024 to Oct 1, 2026.");
    expect(markup).toContain("Show data");
    expect(markup).toContain("No Tokscale usage in the last 30 days");
  });

  it("writes a note instead of a chart with fewer than three dated models", () => {
    const short = { ...timeline, points: timeline.points.slice(0, 2) };
    const markup = text(
      renderToStaticMarkup(
        <ScoreTimeline timeline={short} unit="percent" unitLabel="% correct" benchmarkName="GPQA Diamond" sourceName="Epoch AI" usageAvailable />,
      ),
    );
    expect(markup).toContain("Not enough matched models with a known release date to draw a chart (2 of 3).");
    expect(markup).not.toContain("Show data");
  });
});

describe("BenchmarkIndex", () => {
  it("groups benchmarks, links pages, and keeps link-only benchmarks without a page", () => {
    const markup = renderToStaticMarkup(
      <BenchmarkIndex groups={buildBenchmarkIndex(model)} summary={benchmarkIndexSummary(model)} sources={sourceNotes(model)} />,
    );
    expect(markup).toContain('href="/benchmarks/gpqa-diamond"');
    expect(markup).not.toContain('href="/benchmarks/swe-bench-pro-v2"');
    expect(text(markup)).toContain("Open the SWE-Bench Pro V2 leaderboard");
    expect(text(markup)).toContain("Sources and attribution");
  });
});
