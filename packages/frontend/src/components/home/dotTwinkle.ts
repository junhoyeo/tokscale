/**
 * Pure helpers for the twinkle overlay on the blue dot-matrix art (see
 * scripts/stealth/dither_art.py): 12 px cells, each with a 5 px square dot at
 * offset 3. Kept free of DOM APIs so they can be unit tested.
 */
export const DOT_CELL = 12;
export const DOT_SIZE = 5;
export const DOT_OFFSET = Math.floor((DOT_CELL - DOT_SIZE) / 2);

export interface DotCandidate {
  col: number;
  row: number;
  /** 0..1 brightness of the dot; brighter dots twinkle more often. */
  weight: number;
}

/**
 * Finds the dots in a cell-resolution sample of the art (one RGBA pixel per cell,
 * sampled at the cell center). A dot is opaque at its center; the soft glow
 * between dots is not.
 */
export function findDots(rgba: ArrayLike<number>, cols: number, rows: number, minAlpha = 250): DotCandidate[] {
  const dots: DotCandidate[] = [];
  for (let row = 0; row < rows; row++) {
    for (let col = 0; col < cols; col++) {
      const i = (row * cols + col) * 4;
      if (rgba[i + 3] < minAlpha) continue;
      const luminance = (0.2126 * rgba[i] + 0.7152 * rgba[i + 1] + 0.0722 * rgba[i + 2]) / 255;
      dots.push({ col, row, weight: Math.min(1, Math.max(0.05, luminance)) });
    }
  }
  return dots;
}

/** Brightness of a twinkle at progress p (0..1): rises and falls smoothly, 0 at both ends. */
export function twinkleAlpha(p: number): number {
  if (!(p > 0 && p < 1)) return 0;
  const s = Math.sin(Math.PI * p);
  return s * s;
}

/**
 * Picks a dot index with probability proportional to its weight, using a
 * prefix-sum table and a random number in [0, 1).
 */
export function pickWeighted(prefix: readonly number[], random: number): number {
  const total = prefix[prefix.length - 1] ?? 0;
  if (total <= 0) return -1;
  const target = random * total;
  let lo = 0;
  let hi = prefix.length - 1;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (prefix[mid] > target) hi = mid;
    else lo = mid + 1;
  }
  return lo;
}

export function prefixSums(dots: readonly DotCandidate[]): number[] {
  const out: number[] = [];
  let sum = 0;
  for (const dot of dots) {
    sum += dot.weight * dot.weight;
    out.push(sum);
  }
  return out;
}
