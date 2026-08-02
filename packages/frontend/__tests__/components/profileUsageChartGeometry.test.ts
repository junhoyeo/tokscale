import { describe, expect, it } from "vitest";
import {
  createNonCrossingStackGeometry,
  pointToChartPercent,
  sampleCubicValueSegment,
  type NonCrossingStackGeometry,
} from "../../src/components/profile/usageChartGeometry";

const INTERMEDIATE_SAMPLES = Array.from(
  { length: 65 },
  (_, index) => index / 64,
);

function tolerance(...values: number[]): number {
  return Math.max(1, ...values.map(Math.abs)) * 1e-9;
}

function expectSafeStack(geometry: NonCrossingStackGeometry): void {
  const failures: string[] = [];
  const check = (condition: boolean, message: string) => {
    if (!condition && failures.length < 20) failures.push(message);
  };

  geometry.layers.forEach((layer, layerIndex) => {
    check(
      layer.thickness.values.length === geometry.pointCount,
      `layer ${layerIndex} thickness length`,
    );
    check(
      layer.lower.values.length === geometry.pointCount,
      `layer ${layerIndex} lower length`,
    );
    check(
      layer.upper.values.length === geometry.pointCount,
      `layer ${layerIndex} upper length`,
    );

    layer.thickness.values.forEach((thickness, pointIndex) => {
      const lower = layer.lower.values[pointIndex] ?? 0;
      const upper = layer.upper.values[pointIndex] ?? 0;
      const epsilon = tolerance(thickness, lower, upper);

      check(
        Number.isFinite(thickness),
        `layer ${layerIndex} point ${pointIndex} finite thickness`,
      );
      check(
        Number.isFinite(lower),
        `layer ${layerIndex} point ${pointIndex} finite lower`,
      );
      check(
        Number.isFinite(upper),
        `layer ${layerIndex} point ${pointIndex} finite upper`,
      );
      check(
        thickness >= 0,
        `layer ${layerIndex} point ${pointIndex} nonnegative thickness`,
      );
      check(
        upper + epsilon >= lower,
        `layer ${layerIndex} point ${pointIndex} ordered`,
      );
      check(
        Math.abs(upper - lower - thickness) <= epsilon,
        `layer ${layerIndex} point ${pointIndex} additive`,
      );

      if (layerIndex > 0) {
        const previousUpper =
          geometry.layers[layerIndex - 1]?.upper.values[pointIndex] ?? 0;
        check(
          Math.abs(lower - previousUpper) <= epsilon,
          `layer ${layerIndex} point ${pointIndex} shares boundary`,
        );
      }
    });

    layer.thickness.segments.forEach((thicknessSegment, segmentIndex) => {
      const lowerSegment = layer.lower.segments[segmentIndex];
      const upperSegment = layer.upper.segments[segmentIndex];
      check(
        Boolean(lowerSegment),
        `layer ${layerIndex} segment ${segmentIndex} lower exists`,
      );
      check(
        Boolean(upperSegment),
        `layer ${layerIndex} segment ${segmentIndex} upper exists`,
      );
      if (!lowerSegment || !upperSegment) return;

      check(
        sampleCubicValueSegment(thicknessSegment, 0) === thicknessSegment.from,
        `layer ${layerIndex} segment ${segmentIndex} starts exactly`,
      );
      check(
        sampleCubicValueSegment(thicknessSegment, 1) === thicknessSegment.to,
        `layer ${layerIndex} segment ${segmentIndex} ends exactly`,
      );
      const nextThickness = layer.thickness.segments[segmentIndex + 1];
      if (nextThickness) {
        const outgoing = thicknessSegment.to - thicknessSegment.control2;
        const incoming = nextThickness.control1 - nextThickness.from;
        check(
          Math.abs(outgoing - incoming) <= tolerance(outgoing, incoming),
          `layer ${layerIndex} segment ${segmentIndex} has a continuous tangent`,
        );
      }

      let previousThickness = sampleCubicValueSegment(thicknessSegment, 0);
      INTERMEDIATE_SAMPLES.forEach((progress) => {
        const thickness = sampleCubicValueSegment(thicknessSegment, progress);
        const lower = sampleCubicValueSegment(lowerSegment, progress);
        const upper = sampleCubicValueSegment(upperSegment, progress);
        const epsilon = tolerance(thickness, lower, upper);

        const location = `layer ${layerIndex} segment ${segmentIndex} t=${progress}`;
        check(Number.isFinite(thickness), `${location} finite thickness`);
        check(Number.isFinite(lower), `${location} finite lower`);
        check(Number.isFinite(upper), `${location} finite upper`);
        check(thickness >= 0, `${location} nonnegative thickness`);
        check(upper + epsilon >= lower, `${location} ordered`);
        check(
          Math.abs(upper - lower - thickness) <= epsilon,
          `${location} additive`,
        );

        if (thicknessSegment.to >= thicknessSegment.from) {
          check(
            thickness + epsilon >= previousThickness,
            `${location} monotone increasing`,
          );
        } else {
          check(
            thickness - epsilon <= previousThickness,
            `${location} monotone decreasing`,
          );
        }
        previousThickness = thickness;

        if (layerIndex === geometry.layers.length - 1) {
          check(
            upper <= geometry.maximum + epsilon,
            `${location} inside vertical domain`,
          );
        }
      });
    });
  });

  expect(failures).toEqual([]);
}

describe("non-crossing usage chart geometry", () => {
  it("maps chart coordinates to CSS percentages without inheriting SVG stretch", () => {
    expect(pointToChartPercent(424, 128, 848, 256)).toEqual({
      left: 50,
      top: 50,
    });
    expect(pointToChartPercent(-1, 300, 848, 256)).toEqual({
      left: 0,
      top: 100,
    });
  });

  it("keeps a zero-thickness interval coincident when adjacent spikes would make cumulative curves cross", () => {
    const geometry = createNonCrossingStackGeometry([
      [0, 10, 20, 30],
      [100, 0, 0, 100],
    ]);
    const upperLayer = geometry.layers[1];
    const middleLower = upperLayer?.lower.segments[1];
    const middleUpper = upperLayer?.upper.segments[1];

    expect(middleLower).toBeDefined();
    expect(middleUpper).toBeDefined();
    if (!middleLower || !middleUpper) return;
    let maximumDifference = 0;
    for (let sample = 0; sample <= 1_000; sample += 1) {
      const progress = sample / 1_000;
      maximumDifference = Math.max(
        maximumDifference,
        Math.abs(
          sampleCubicValueSegment(middleUpper, progress) -
            sampleCubicValueSegment(middleLower, progress),
        ),
      );
    }
    expect(maximumDifference).toBeLessThanOrEqual(1e-12);
    expectSafeStack(geometry);
  });

  it("keeps pathological spikes and opposing zero transitions separated between every sample", () => {
    const geometry = createNonCrossingStackGeometry([
      [0, 1_000_000_000, 0, 0, 9, 0, 700, 0, 0, 2, 0, 0],
      [1_000_000_000, 0, 0, 12, 0, 800, 0, 0, 3, 0, 0, 50],
      [0, 0, 600_000_000, 0, 0, 0, 1, 0, 400, 0, 5, 0],
      [25, 25, 25, 0, 25, 25, 0, 25, 25, 25, 0, 25],
    ]);

    expect(geometry.pointCount).toBe(12);
    expect(geometry.layers).toHaveLength(4);
    expectSafeStack(geometry);
  });

  it("preserves smooth non-crossing boundaries for real-ish multi-model activity", () => {
    let seed = 0x5eed1234;
    const random = () => {
      seed = (seed * 1664525 + 1013904223) >>> 0;
      return seed / 0x1_0000_0000;
    };
    const series = Array.from({ length: 18 }, (_, seriesIndex) =>
      Array.from({ length: 120 }, (_, dayIndex) => {
        const trend = Math.max(0, dayIndex - seriesIndex * 2) * 130;
        const weekly = (Math.sin((dayIndex + seriesIndex) / 5) + 1) * 900;
        const burst = random() > 0.94 ? random() * 150_000 : 0;
        const inactive = random() < 0.12;
        return inactive ? 0 : Math.round(trend + weekly + burst);
      }),
    );
    const geometry = createNonCrossingStackGeometry(series, 120);

    expect(geometry.maximum).toBeGreaterThan(0);
    expectSafeStack(geometry);
  });

  it("normalizes malformed values and mismatched lengths without non-finite controls", () => {
    const geometry = createNonCrossingStackGeometry(
      [
        [Number.NaN, -10, Number.POSITIVE_INFINITY, 40],
        [5, Number.NEGATIVE_INFINITY],
        [],
      ],
      6,
    );

    expect(geometry.layers[0]?.thickness.values).toEqual([0, 0, 0, 40, 0, 0]);
    expect(geometry.layers[1]?.thickness.values).toEqual([5, 0, 0, 0, 0, 0]);
    expect(geometry.layers[2]?.thickness.values).toEqual([0, 0, 0, 0, 0, 0]);
    expectSafeStack(geometry);

    expect(createNonCrossingStackGeometry([], Number.NaN)).toEqual({
      layers: [],
      pointCount: 0,
      maximum: 0,
    });
    const singleton = createNonCrossingStackGeometry([[7]], 1);
    expect(singleton.layers[0]?.upper.values).toEqual([7]);
    expect(singleton.layers[0]?.upper.segments).toEqual([]);
    expect(singleton.maximum).toBe(7);
  });

  it("saturates cumulative boundaries when finite series would overflow", () => {
    const geometry = createNonCrossingStackGeometry([
      [Number.MAX_VALUE, Number.MAX_VALUE],
      [Number.MAX_VALUE, Number.MAX_VALUE],
    ]);

    expect(geometry.layers[1]?.upper.values[0]).toBe(Number.MAX_VALUE);
    expect(Number.isFinite(geometry.maximum)).toBe(true);
    for (const progress of [0, 0.25, 0.5, 0.75, 1]) {
      const segment = geometry.layers[1]?.upper.segments[0];
      expect(segment).toBeDefined();
      expect(
        Number.isFinite(
          segment ? sampleCubicValueSegment(segment, progress) : NaN,
        ),
      ).toBe(true);
    }
  });
});
