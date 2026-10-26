import { describe, expect, it } from "vitest";
import { linearDomain, linearPosition, linearTicks, logDomain, logPosition, logTicks, paretoFrontier } from "@/lib/benchmarks/charts";

describe("chart scales", () => {
  it("positions values on log and linear scales, clamped to 0..1", () => {
    expect(logPosition(10, 1, 100)).toBeCloseTo(0.5, 10);
    expect(logPosition(1000, 1, 100)).toBe(1);
    expect(logPosition(0, 1, 100)).toBe(0);
    expect(linearPosition(160, 150, 170)).toBe(0.5);
    expect(linearPosition(5, 5, 5)).toBe(0.5);
  });

  it("pads domains to decades and steps", () => {
    expect(logDomain([3, 165])).toEqual([1, 1000]);
    expect(logDomain([0, -1])).toBeNull();
    expect(linearDomain([152.3, 168.8], 5)).toEqual([150, 170]);
    expect(linearDomain([], 5)).toBeNull();
  });

  it("builds ticks", () => {
    expect(logTicks(1, 1000)).toEqual([1, 10, 100, 1000]);
    expect(logTicks(0.1, 10)).toEqual([0.1, 0.2, 0.5, 1, 2, 5, 10]);
    expect(linearTicks(150, 170, 5)).toEqual([150, 155, 160, 165, 170]);
  });
});

describe("paretoFrontier", () => {
  it("keeps the points that are cheaper than every better point", () => {
    const frontier = paretoFrontier([
      { key: "cheap-weak", x: 0.1, y: 150 },
      { key: "cheap-better", x: 0.2, y: 160 },
      { key: "dominated", x: 0.5, y: 155 },
      { key: "best", x: 1.4, y: 168.8 },
      { key: "pricey-worse", x: 2, y: 165 },
    ]);
    expect(frontier.map((p) => p.key)).toEqual(["cheap-weak", "cheap-better", "best"]);
  });

  it("keeps the higher score when prices tie and ignores non-finite points", () => {
    const frontier = paretoFrontier([
      { key: "a", x: 1, y: 150 },
      { key: "b", x: 1, y: 155 },
      { key: "c", x: Number.NaN, y: 200 },
    ]);
    expect(frontier.map((p) => p.key)).toEqual(["b"]);
  });
});
