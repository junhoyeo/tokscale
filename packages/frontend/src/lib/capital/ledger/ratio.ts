import type { Amount, Basis } from "./schema";

type RatioBasis = "exact" | "approximately" | "more_than" | "at_least" | "less_than" | "up_to";
export interface ValuationRatio {
  /** Ratio of the stated values or bound thresholds, not necessarily an exact multiple. */
  ratio: number;
  basis: RatioBasis;
  short: string;
}

type Direction = "point" | "lower" | "upper";
function direction(basis: Basis): Direction | null {
  if (basis === "exact" || basis === "approximately") return "point";
  if (basis === "more_than" || basis === "at_least") return "lower";
  if (basis === "up_to") return "upper";
  // A range has no endpoints in this schema. An approximate ceiling has no strict bound.
  return null;
}

/** Divide positive valuations without turning a lower/upper bound into an exact step-up. */
export function valuationRatio(
  numerator: Pick<Amount, "value" | "basis" | "currency">,
  denominator: Pick<Amount, "value" | "basis" | "currency">,
): ValuationRatio | null {
  if (numerator.value === null || denominator.value === null || numerator.value <= 0 || denominator.value <= 0) return null;
  if (!Number.isFinite(numerator.value) || !Number.isFinite(denominator.value) || numerator.currency !== denominator.currency) return null;
  const top = direction(numerator.basis);
  const bottom = direction(denominator.basis);
  if (!top || !bottom) return null;
  const approximate = numerator.basis === "approximately" || denominator.basis === "approximately";
  let basis: RatioBasis;
  if (top === "point" && bottom === "point") {
    basis = approximate ? "approximately" : "exact";
  } else if (approximate) {
    // An approximate point cannot establish a strict bound for another bounded amount.
    return null;
  } else if ((top === "lower" && bottom !== "lower") || (top === "point" && bottom === "upper")) {
    basis = numerator.basis === "more_than" ? "more_than" : "at_least";
  } else if ((top === "upper" && bottom !== "upper") || (top === "point" && bottom === "lower")) {
    basis = denominator.basis === "more_than" ? "less_than" : "up_to";
  } else {
    // Two lower bounds or two upper bounds do not bound their quotient at this threshold.
    return null;
  }
  const ratio = numerator.value / denominator.value;
  if (!Number.isFinite(ratio * 100)) return null;
  const symbols: Record<RatioBasis, string> = { exact: "", approximately: "≈", more_than: ">", at_least: "≥", less_than: "<", up_to: "≤" };
  // Round bounds outward: >1.795 must not become >1.80; <1.791 must not become <1.79.
  const displayed = basis === "more_than" || basis === "at_least"
    ? Math.floor(ratio * 100) / 100
    : basis === "less_than" || basis === "up_to"
      ? Math.ceil(ratio * 100) / 100
      : ratio;
  return { ratio, basis, short: `${symbols[basis]}${displayed.toFixed(2)}x` };
}
