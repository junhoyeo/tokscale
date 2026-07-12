export interface CubicValueSegment {
  index: number;
  from: number;
  control1: number;
  control2: number;
  to: number;
}

export interface CubicValueBoundary {
  values: number[];
  segments: CubicValueSegment[];
}

export interface NonCrossingStackLayer {
  /** The independently smoothed, nonnegative thickness of this layer. */
  thickness: CubicValueBoundary;
  /** The cumulative boundary below this layer. */
  lower: CubicValueBoundary;
  /** The cumulative boundary above this layer. */
  upper: CubicValueBoundary;
}

export interface NonCrossingStackGeometry {
  layers: NonCrossingStackLayer[];
  pointCount: number;
  /** A safe vertical-domain maximum that includes all cubic control values. */
  maximum: number;
}

function finiteNonnegative(value: number | undefined): number {
  return Number.isFinite(value) ? Math.max(0, value ?? 0) : 0;
}

function normalizedPointCount(
  seriesValues: readonly (readonly number[])[],
  pointCount: number | undefined,
): number {
  if (pointCount === undefined) {
    return seriesValues.reduce(
      (maximum, values) => Math.max(maximum, values.length),
      0,
    );
  }
  return Number.isFinite(pointCount) ? Math.max(0, Math.floor(pointCount)) : 0;
}

function normalizeValues(
  values: readonly number[],
  pointCount: number,
): number[] {
  return Array.from({ length: pointCount }, (_, index) =>
    finiteNonnegative(values[index]),
  );
}

function saturatingAdd(left: number, right: number): number {
  const sum = left + right;
  return Number.isFinite(sum) ? sum : Number.MAX_VALUE;
}

function clamp(value: number, minimum: number, maximum: number): number {
  return Math.max(minimum, Math.min(maximum, value));
}

export function pointToChartPercent(
  x: number,
  y: number,
  width: number,
  height: number,
): { left: number; top: number } {
  const safeWidth = Number.isFinite(width) && width > 0 ? width : 1;
  const safeHeight = Number.isFinite(height) && height > 0 ? height : 1;
  const safeX = Number.isFinite(x) ? x : 0;
  const safeY = Number.isFinite(y) ? y : 0;
  return {
    left: clamp((safeX / safeWidth) * 100, 0, 100),
    top: clamp((safeY / safeHeight) * 100, 0, 100),
  };
}

/**
 * Smooth one nonnegative series with a Fritsch-Carlson monotone cubic.
 *
 * Keeping every Bézier ordinate inside its adjacent endpoint range makes the
 * thickness finite and nonnegative for the entire segment, not only at the
 * sampled dates. Those thickness curves can therefore be added component by
 * component to create cumulative stack boundaries that cannot cross.
 */
function monotoneBoundary(values: readonly number[]): CubicValueBoundary {
  const normalized = values.map(finiteNonnegative);
  if (normalized.length < 2) {
    return { values: normalized, segments: [] };
  }

  const slopes = normalized.slice(0, -1).map((value, index) => {
    const slope = (normalized[index + 1] ?? 0) - value;
    return Number.isFinite(slope) ? slope : 0;
  });
  const tangents = normalized.map((_, index) => {
    if (index === 0) return slopes[0] ?? 0;
    if (index === normalized.length - 1) return slopes.at(-1) ?? 0;
    const before = slopes[index - 1] ?? 0;
    const after = slopes[index] ?? 0;
    return before * after <= 0 ? 0 : (before + after) / 2;
  });

  for (let index = 0; index < slopes.length; index += 1) {
    const slope = slopes[index] ?? 0;
    if (slope === 0) {
      tangents[index] = 0;
      tangents[index + 1] = 0;
      continue;
    }

    const left = (tangents[index] ?? 0) / slope;
    const right = (tangents[index + 1] ?? 0) / slope;
    const magnitude = Math.hypot(left, right);
    if (magnitude > 3) {
      const scale = 3 / magnitude;
      tangents[index] = scale * left * slope;
      tangents[index + 1] = scale * right * slope;
    }
  }

  const segments = normalized.slice(0, -1).map((from, index) => {
    const to = normalized[index + 1] ?? 0;
    const minimum = Math.min(from, to);
    const maximum = Math.max(from, to);
    return {
      index,
      from,
      control1: clamp(from + (tangents[index] ?? 0) / 3, minimum, maximum),
      control2: clamp(to - (tangents[index + 1] ?? 0) / 3, minimum, maximum),
      to,
    };
  });

  return { values: normalized, segments };
}

function addBoundaries(
  lower: CubicValueBoundary,
  thickness: CubicValueBoundary,
): CubicValueBoundary {
  return {
    values: lower.values.map((value, index) =>
      saturatingAdd(value, thickness.values[index] ?? 0),
    ),
    segments: lower.segments.map((segment, index) => {
      const addition = thickness.segments[index];
      return {
        index: segment.index,
        from: saturatingAdd(segment.from, addition?.from ?? 0),
        control1: saturatingAdd(segment.control1, addition?.control1 ?? 0),
        control2: saturatingAdd(segment.control2, addition?.control2 ?? 0),
        to: saturatingAdd(segment.to, addition?.to ?? 0),
      };
    }),
  };
}

function boundaryMaximum(boundary: CubicValueBoundary): number {
  return Math.max(
    0,
    ...boundary.values,
    ...boundary.segments.flatMap(({ control1, control2 }) => [
      control1,
      control2,
    ]),
  );
}

/**
 * Build compatible cubic boundaries for a stacked area chart.
 *
 * Each layer's upper curve is the exact Bézier sum of its lower curve and a
 * separately monotone-smoothed, nonnegative thickness curve. Since Bernstein
 * basis weights are nonnegative, `upper(t) - lower(t)` cannot be negative at
 * any point between samples.
 */
export function createNonCrossingStackGeometry(
  seriesValues: readonly (readonly number[])[],
  requestedPointCount?: number,
): NonCrossingStackGeometry {
  const pointCount = normalizedPointCount(seriesValues, requestedPointCount);
  let lower = monotoneBoundary(Array.from({ length: pointCount }, () => 0));
  const layers = seriesValues.map((values) => {
    const thickness = monotoneBoundary(normalizeValues(values, pointCount));
    const upper = addBoundaries(lower, thickness);
    const layer = { thickness, lower, upper };
    lower = upper;
    return layer;
  });

  return {
    layers,
    pointCount,
    maximum: boundaryMaximum(lower),
  };
}

export function sampleCubicValueSegment(
  segment: CubicValueSegment,
  progress: number,
): number {
  const t = clamp(Number.isFinite(progress) ? progress : 0, 0, 1);
  const interpolate = (left: number, right: number): number =>
    left + (right - left) * t;
  const first = interpolate(segment.from, segment.control1);
  const second = interpolate(segment.control1, segment.control2);
  const third = interpolate(segment.control2, segment.to);
  return interpolate(interpolate(first, second), interpolate(second, third));
}
