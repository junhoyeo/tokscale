import { describe, expect, it } from "vitest";
import { benchmarkData, BENCHMARKS } from "@/lib/benchmarks";
import { validateBenchmarkContent } from "@/lib/benchmarks/content";
import epochJson from "@/content/benchmarks/epoch.json";
import modelsDevJson from "@/content/benchmarks/models-dev.json";
import reportJson from "@/content/benchmarks/report.json";
import { aliasFileSchema } from "@/lib/benchmarks/schema";
import aliasesJson from "@/content/benchmarks/aliases.json";

describe("committed benchmark snapshots", () => {
  it("validate, and every benchmark with a source has results", () => {
    for (const def of BENCHMARKS) {
      const resolved = benchmarkData.get(def.slug);
      expect(resolved).not.toBeNull();
      if (def.source) {
        expect(resolved?.results.length).toBeGreaterThan(0);
        expect(resolved?.snapshot?.retrievedAt).toMatch(/^\d{4}-\d{2}-\d{2}T/);
      } else {
        expect(resolved?.results).toEqual([]);
      }
    }
  });

  it("never stores a model key without a mapping method", () => {
    for (const b of benchmarkData.list()) {
      for (const r of b.results) expect(r.modelKey === null).toBe(r.mapping === null);
    }
  });

  it("keeps Terminal-Bench rows linked to the leaderboard", () => {
    for (const r of benchmarkData.get("terminal-bench-4")?.results ?? []) {
      expect(r.sourceUrl).toBe("https://www.tbench.ai/leaderboard");
    }
  });

  it("has a valid alias map", () => {
    expect(() => aliasFileSchema.parse(aliasesJson)).not.toThrow();
  });

  it("rejects a snapshot whose benchmark is missing", () => {
    const broken = { ...epochJson, benchmarks: epochJson.benchmarks.filter((b) => b.slug !== "gpqa-diamond") };
    expect(() => validateBenchmarkContent({ sources: [broken], catalog: modelsDevJson, report: reportJson })).toThrow(/gpqa-diamond|lmarena|tbench/);
  });
});
