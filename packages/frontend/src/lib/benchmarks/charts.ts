/**
 * Geometry for the dashboard charts: log scales, ticks and the price
 * frontier. Pure functions, unit-tested.
 */

export interface ChartPoint {
  key: string;
  x: number;
  y: number;
}

/** Map a positive value to 0..1 on a log10 scale over [min, max]. */
export function logPosition(value: number, min: number, max: number): number {
  if (!(value > 0) || !(min > 0) || !(max > min)) return 0;
  const t = (Math.log10(value) - Math.log10(min)) / (Math.log10(max) - Math.log10(min));
  return Math.min(1, Math.max(0, t));
}

/** Map a value to 0..1 on a linear scale over [min, max]. */
export function linearPosition(value: number, min: number, max: number): number {
  if (!(max > min)) return 0.5;
  return Math.min(1, Math.max(0, (value - min) / (max - min)));
}

/** Powers of ten (and 2x, 5x when the range is short) covering [min, max]. */
export function logTicks(min: number, max: number): number[] {
  if (!(min > 0) || !(max >= min)) return [];
  const low = Math.floor(Math.log10(min));
  const high = Math.ceil(Math.log10(max));
  const steps = high - low <= 2 ? [1, 2, 5] : [1];
  const ticks: number[] = [];
  for (let e = low; e <= high; e++) {
    for (const s of steps) {
      const v = s * 10 ** e;
      if (v >= min * 0.999 && v <= max * 1.001) ticks.push(Number(v.toPrecision(6)));
    }
  }
  return ticks;
}

/** Log domain padded to whole decades around the data. */
export function logDomain(values: readonly number[]): [number, number] | null {
  const positive = values.filter((v) => v > 0 && Number.isFinite(v));
  if (positive.length === 0) return null;
  const min = 10 ** Math.floor(Math.log10(Math.min(...positive)));
  const max = 10 ** Math.ceil(Math.log10(Math.max(...positive)));
  return [min, max === min ? min * 10 : max];
}

/** Linear domain padded to `step` multiples around the data. */
export function linearDomain(values: readonly number[], step: number): [number, number] | null {
  const finite = values.filter((v) => Number.isFinite(v));
  if (finite.length === 0) return null;
  const min = Math.floor(Math.min(...finite) / step) * step;
  const max = Math.ceil(Math.max(...finite) / step) * step;
  return [min, max === min ? min + step : max];
}

export function linearTicks(min: number, max: number, step: number): number[] {
  const ticks: number[] = [];
  for (let v = min; v <= max + step * 1e-9; v += step) ticks.push(Number(v.toFixed(6)));
  return ticks;
}

/**
 * Price frontier: the points no other point beats on both price and score.
 * Sorted by price (x) ascending, each frontier point has a higher score (y)
 * than every cheaper point. Ties on price keep the higher score.
 */
export function paretoFrontier(points: readonly ChartPoint[]): ChartPoint[] {
  const sorted = [...points]
    .filter((p) => Number.isFinite(p.x) && Number.isFinite(p.y))
    .sort((a, b) => a.x - b.x || b.y - a.y || (a.key < b.key ? -1 : 1));
  const frontier: ChartPoint[] = [];
  let best = Number.NEGATIVE_INFINITY;
  for (const point of sorted) {
    if (point.y > best) {
      frontier.push(point);
      best = point.y;
    }
  }
  return frontier;
}
