import { describe, expect, it } from "vitest";
import { addMix, effectivePricePerMillion, emptyMix, mixShares, mixTotal, threeToOneBlend } from "@/lib/benchmarks/price";
import { foldModelMixes } from "@/lib/benchmarks/usage";
import type { UsageFact } from "@/lib/explore/rollup";
import type { ExploreWindow } from "@/lib/explore/window";

const opus = { input: 4, output: 20, cacheRead: 0.2, cacheWrite: 5 };

describe("effective price at a token mix", () => {
  it("weights each bucket's list price by its share of tokens", () => {
    // 98.2% cache reads, as measured for Claude Opus 5.5 in the local seed.
    const mix = { input: 0.5, output: 0.3, cacheRead: 98.2, cacheWrite: 1.0, reasoning: 0 };
    const price = effectivePricePerMillion(opus, mix);
    expect(price?.perMillion).toBeCloseTo(0.005 * 4 + 0.003 * 20 + 0.982 * 0.2 + 0.01 * 5, 10);
    expect(price?.assumptions).toEqual([]);
  });

  it("works the same for absolute counts and shares", () => {
    const counts = { input: 1_000, output: 500, cacheRead: 90_000, cacheWrite: 8_000, reasoning: 500 };
    const shares = mixShares(counts);
    expect(effectivePricePerMillion(opus, counts)?.perMillion).toBeCloseTo(effectivePricePerMillion(opus, shares)?.perMillion ?? -1, 10);
  });

  it("prices reasoning tokens as output", () => {
    const reasoningOnly = { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, reasoning: 10 };
    expect(effectivePricePerMillion(opus, reasoningOnly)?.perMillion).toBe(20);
  });

  it("falls back to the input price for missing cache prices and says so", () => {
    const noCache = { input: 2, output: 8, cacheRead: null, cacheWrite: null };
    const mix = { input: 50, output: 0, cacheRead: 25, cacheWrite: 25, reasoning: 0 };
    const price = effectivePricePerMillion(noCache, mix);
    expect(price?.perMillion).toBe(2);
    expect(price?.assumptions).toHaveLength(2);
  });

  it("does not need a cache price when the mix has no cache tokens", () => {
    const price = effectivePricePerMillion({ input: 1, output: 3, cacheRead: null, cacheWrite: null }, { input: 3, output: 1, cacheRead: 0, cacheWrite: 0, reasoning: 0 });
    expect(price).toEqual({ perMillion: 1.5, assumptions: [] });
  });

  it("returns null for an empty mix or a missing input or output price", () => {
    expect(effectivePricePerMillion(opus, emptyMix())).toBeNull();
    expect(effectivePricePerMillion({ ...opus, output: null }, { input: 1, output: 1, cacheRead: 0, cacheWrite: 0, reasoning: 0 })).toBeNull();
    expect(effectivePricePerMillion({ ...opus, input: null }, { input: 1, output: 0, cacheRead: 0, cacheWrite: 0, reasoning: 0 })).toBeNull();
  });

  it("ignores negative and non-finite counts", () => {
    const mix = { input: -5, output: Number.NaN, cacheRead: 10, cacheWrite: 0, reasoning: Number.POSITIVE_INFINITY };
    expect(mixTotal(mix)).toBe(10);
    expect(effectivePricePerMillion(opus, mix)?.perMillion).toBeCloseTo(0.2, 10);
  });

  it("computes the usual 3:1 blend for comparison", () => {
    expect(threeToOneBlend(opus)).toBe(8);
    expect(threeToOneBlend({ ...opus, output: null })).toBeNull();
  });

  it("adds mixes bucket by bucket", () => {
    const total = addMix(emptyMix(), { input: 1, output: 2, cacheRead: 3, cacheWrite: 4, reasoning: 5 });
    addMix(total, { input: 1, output: 1, cacheRead: 1, cacheWrite: 1, reasoning: 1 });
    expect(total).toEqual({ input: 2, output: 3, cacheRead: 4, cacheWrite: 5, reasoning: 6 });
  });
});

describe("foldModelMixes", () => {
  const window: ExploreWindow = { period: "30d", start: "2026-09-20", end: "2026-10-19", previous: { start: "2026-08-21", end: "2026-09-19" }, days: 30 };
  const fact = (model: string, current: boolean, cacheRead: number): UsageFact => ({
    userId: "u1",
    client: "claude",
    model,
    current,
    tokens: 10 + cacheRead,
    cost: 0,
    input: 10,
    output: 0,
    cacheRead,
    cacheWrite: 0,
    reasoning: 0,
  });

  it("sums the current window per canonical key and skips previous-window and unidentified rows", () => {
    const rollup = foldModelMixes(window, [
      fact("claude-opus-5-5", true, 90),
      fact("anthropic/claude-opus-5.5", true, 10),
      fact("claude-opus-5-5", false, 1_000),
      fact("<synthetic>", true, 500),
    ]);
    expect(rollup.byModel["claude-opus-5-5"]).toEqual({ input: 20, output: 0, cacheRead: 100, cacheWrite: 0, reasoning: 0 });
    expect(rollup.community).toEqual({ input: 20, output: 0, cacheRead: 100, cacheWrite: 0, reasoning: 0 });
    expect(Object.keys(rollup.byModel)).toEqual(["claude-opus-5-5"]);
    expect(rollup).toMatchObject({ period: "30d", start: "2026-09-20", end: "2026-10-19" });
  });
});
