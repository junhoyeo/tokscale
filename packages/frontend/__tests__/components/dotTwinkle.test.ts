import { describe, expect, it } from "vitest";
import { DOT_CELL, DOT_OFFSET, DOT_SIZE, findDots, pickWeighted, prefixSums, twinkleAlpha } from "@/components/home/dotTwinkle";

describe("dot twinkle helpers", () => {
  it("matches the cell layout of the dither script", () => {
    expect([DOT_CELL, DOT_SIZE, DOT_OFFSET]).toEqual([12, 5, 3]);
  });

  it("finds opaque dot centers and weights them by brightness", () => {
    // 3 x 1 cells: an opaque light dot, glow only (not a dot), an opaque dark dot
    const rgba = [200, 220, 255, 255, 20, 40, 120, 90, 20, 60, 140, 255];
    const dots = findDots(rgba, 3, 1);
    expect(dots.map((d) => d.col)).toEqual([0, 2]);
    expect(dots[0].weight).toBeGreaterThan(dots[1].weight);
  });

  it("fades a twinkle in and out", () => {
    expect(twinkleAlpha(0)).toBe(0);
    expect(twinkleAlpha(1)).toBe(0);
    expect(twinkleAlpha(0.5)).toBeCloseTo(1, 5);
    expect(twinkleAlpha(-1)).toBe(0);
    expect(twinkleAlpha(Number.NaN)).toBe(0);
  });

  it("picks dots in proportion to their weight", () => {
    const prefix = prefixSums([
      { col: 0, row: 0, weight: 0.1 },
      { col: 1, row: 0, weight: 1 },
    ]);
    expect(pickWeighted(prefix, 0)).toBe(0);
    expect(pickWeighted(prefix, 0.5)).toBe(1);
    expect(pickWeighted(prefix, 0.999)).toBe(1);
    expect(pickWeighted([], 0.5)).toBe(-1);
  });
});
