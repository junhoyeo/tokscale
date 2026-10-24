/**
 * Price at real agent usage.
 *
 * Effective price = list price per bucket weighted by a token mix:
 *
 *   (input * p_in + (output + reasoning) * p_out + cacheRead * p_cr + cacheWrite * p_cw)
 *   / (input + output + reasoning + cacheRead + cacheWrite)
 *
 * Prices are USD per 1M tokens from models.dev. Reasoning tokens are priced
 * as output, the same way Tokscale's own cost estimate prices them. The
 * result is always an estimate at list prices: discounts, batch pricing and
 * long-context tiers are not applied.
 */

export interface ListPrice {
  /** USD per 1M tokens; null when the provider lists no price for the bucket. */
  input: number | null;
  output: number | null;
  cacheRead: number | null;
  cacheWrite: number | null;
}

export interface MixCounts {
  input: number;
  output: number;
  cacheRead: number;
  cacheWrite: number;
  reasoning: number;
}

export interface EffectivePrice {
  /** USD per 1M tokens at the mix. */
  perMillion: number;
  /** Plain sentences describing every assumption the number depends on. */
  assumptions: string[];
}

function safe(value: number): number {
  return Number.isFinite(value) && value > 0 ? value : 0;
}

function validPrice(value: number | null): value is number {
  return value !== null && Number.isFinite(value) && value >= 0;
}

export function mixTotal(mix: MixCounts): number {
  return safe(mix.input) + safe(mix.output) + safe(mix.cacheRead) + safe(mix.cacheWrite) + safe(mix.reasoning);
}

/** Shares of the five buckets (0..1 each); all zero for an empty mix. */
export function mixShares(mix: MixCounts): MixCounts {
  const total = mixTotal(mix);
  if (total <= 0) return { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, reasoning: 0 };
  return {
    input: safe(mix.input) / total,
    output: safe(mix.output) / total,
    cacheRead: safe(mix.cacheRead) / total,
    cacheWrite: safe(mix.cacheWrite) / total,
    reasoning: safe(mix.reasoning) / total,
  };
}

export function addMix(target: MixCounts, source: MixCounts): MixCounts {
  target.input += safe(source.input);
  target.output += safe(source.output);
  target.cacheRead += safe(source.cacheRead);
  target.cacheWrite += safe(source.cacheWrite);
  target.reasoning += safe(source.reasoning);
  return target;
}

export function emptyMix(): MixCounts {
  return { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, reasoning: 0 };
}

/**
 * Effective USD per 1M tokens of `price` at `mix` (counts or shares).
 * Returns null when the mix is empty or when input or output tokens are
 * present but the provider lists no price for them. A missing cache price
 * falls back to the input price and says so in `assumptions`.
 */
export function effectivePricePerMillion(price: ListPrice, mix: MixCounts): EffectivePrice | null {
  const shares = mixShares(mix);
  const total = shares.input + shares.output + shares.cacheRead + shares.cacheWrite + shares.reasoning;
  if (total <= 0) return null;
  const outputShare = shares.output + shares.reasoning;
  if (shares.input > 0 && !validPrice(price.input)) return null;
  if (outputShare > 0 && !validPrice(price.output)) return null;

  const assumptions: string[] = [];
  let cacheRead = price.cacheRead;
  if (shares.cacheRead > 0 && !validPrice(cacheRead)) {
    if (!validPrice(price.input)) return null;
    cacheRead = price.input;
    assumptions.push("No cache-read price is listed, so cache reads are priced as input.");
  }
  let cacheWrite = price.cacheWrite;
  if (shares.cacheWrite > 0 && !validPrice(cacheWrite)) {
    if (!validPrice(price.input)) return null;
    cacheWrite = price.input;
    assumptions.push("No cache-write price is listed, so cache writes are priced as input.");
  }

  const perMillion =
    shares.input * (price.input ?? 0) +
    outputShare * (price.output ?? 0) +
    shares.cacheRead * (cacheRead ?? 0) +
    shares.cacheWrite * (cacheWrite ?? 0);
  return { perMillion, assumptions };
}

/** The usual 3:1 input/output blend, for comparison only. Null without both prices. */
export function threeToOneBlend(price: ListPrice): number | null {
  if (!validPrice(price.input) || !validPrice(price.output)) return null;
  return (3 * price.input + price.output) / 4;
}
