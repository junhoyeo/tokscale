import { SUMMABLE_MEASURES, type Amount, type Basis, type Measure, type Quantity, type QuantityMeasure } from "./schema";

/**
 * The ledger guard. Every money total on a Capital page goes through `total()`.
 *
 * A total exists only inside one measure, one currency and one basis family:
 * - "point":   exact or approximate figures ("$65 billion", "about $17.4 billion")
 * - "ceiling": stated maximums ("up to $6.5 billion")
 * - "floor":   stated minimums ("more than $100 billion")
 * Ranges never sum. Valuations, marks, run-rates, cumulative statements,
 * guidance, estimates and per-share prices never sum (see SUMMABLE_MEASURES).
 * Press-reported, estimated and undisclosed amounts never enter a total.
 * Amounts already counted elsewhere are skipped, not added:
 * - includedIn: a later event counts this money (Amazon's $5B inside Series H)
 * - partOf: a component of a larger amount in the same event
 * - alternativeTo: another figure for the same thing ("about $19.4 billion" with options)
 *
 * Mixing is a bug in the caller, so it throws instead of returning a number.
 */

export type BasisFamily = "point" | "ceiling" | "floor";

export function basisFamily(basis: Basis): BasisFamily | null {
  switch (basis) {
    case "exact":
    case "approximately":
      return "point";
    case "up_to":
    case "up_to_approximately":
      return "ceiling";
    case "more_than":
    case "at_least":
      return "floor";
    case "range":
      return null;
  }
}

export class LedgerTotalError extends Error {
  constructor(message: string) {
    super(`[capital ledger total] ${message}`);
    this.name = "LedgerTotalError";
  }
}

export type SkipReason = "included_in" | "part_of" | "alternative";

export interface LedgerTotal {
  measure: Measure;
  currency: string;
  /** Basis family of the counted amounts; null when nothing was counted. */
  family: BasisFamily | null;
  /** Sum of the counted amounts; null when nothing was counted (never 0 for "nothing"). */
  value: number | null;
  /** True when any counted amount is approximate. */
  approximate: boolean;
  counted: Amount[];
  skipped: Array<{ amount: Amount; reason: SkipReason }>;
}

/** Statuses that may enter a total. "reported" (press), "estimated" and "undisclosed" never do. */
const TOTAL_STATUSES = new Set(["announced", "signed", "closed", "filed"]);

export function skipReason(amount: Amount): SkipReason | null {
  if (amount.includedIn) return "included_in";
  if (amount.partOf) return "part_of";
  if (amount.alternativeTo) return "alternative";
  return null;
}

export function total(amounts: readonly Amount[], expected: { measure: Measure; currency: string }): LedgerTotal {
  const { measure, currency } = expected;
  if (!SUMMABLE_MEASURES.has(measure)) {
    throw new LedgerTotalError(`${measure} cannot be summed (it is a level, a mark, a run-rate, a guidance or an estimate)`);
  }
  const counted: Amount[] = [];
  const skipped: LedgerTotal["skipped"] = [];
  let family: BasisFamily | null = null;
  let approximate = false;
  let value = 0;

  for (const amount of amounts) {
    if (amount.measure !== measure) {
      throw new LedgerTotalError(`mixed measures: expected ${measure}, got ${amount.measure} (${amount.id})`);
    }
    if (amount.currency !== currency) {
      throw new LedgerTotalError(`mixed currencies: expected ${currency}, got ${amount.currency} (${amount.id})`);
    }
    if (!TOTAL_STATUSES.has(amount.status) || amount.value === null) {
      throw new LedgerTotalError(`amount ${amount.id} has status "${amount.status}", which never enters a total`);
    }
    const reason = skipReason(amount);
    if (reason) {
      skipped.push({ amount, reason });
      continue;
    }
    const amountFamily = basisFamily(amount.basis);
    if (amountFamily === null) throw new LedgerTotalError(`amount ${amount.id} is a range and cannot be summed`);
    if (family !== null && family !== amountFamily) {
      throw new LedgerTotalError(`mixed bases: ${family} and ${amountFamily} (${amount.id})`);
    }
    family = amountFamily;
    approximate ||= amount.basis === "approximately" || amount.basis === "up_to_approximately";
    value += amount.value;
    counted.push(amount);
  }

  return { measure, currency, family, value: counted.length > 0 ? value : null, approximate, counted, skipped };
}

/** One total per currency, largest first. Currencies are never converted or added together. */
export function totalsByCurrency(amounts: readonly Amount[], measure: Measure): LedgerTotal[] {
  const byCurrency = new Map<string, Amount[]>();
  for (const amount of amounts) {
    const list = byCurrency.get(amount.currency) ?? [];
    list.push(amount);
    byCurrency.set(amount.currency, list);
  }
  return [...byCurrency.entries()]
    .map(([currency, list]) => total(list, { measure, currency }))
    .sort((a, b) => (b.value ?? 0) - (a.value ?? 0) || a.currency.localeCompare(b.currency));
}

/** Like total(), but reports why a set cannot be summed instead of throwing. For UI fallbacks ("N deals, largest ..."). */
export function tryTotal(
  amounts: readonly Amount[],
  expected: { measure: Measure; currency: string }
): { ok: true; total: LedgerTotal } | { ok: false; reason: string } {
  try {
    return { ok: true, total: total(amounts, expected) };
  } catch (error) {
    if (error instanceof LedgerTotalError) return { ok: false, reason: error.message };
    throw error;
  }
}

/**
 * Capacity follows the same rule as money: one measure (critical IT MW, GW, GPUs) and one basis
 * family. "Up to 5 GW" plus "about 3.5 GW" is not a number, so mixed sets are not added.
 */
export function tryQuantityTotal(
  quantities: ReadonlyArray<Pick<Quantity, "measure" | "value" | "basis">>,
  measure: QuantityMeasure
):
  | { ok: true; value: number; family: BasisFamily; approximate: boolean; count: number }
  | { ok: false; reason: string; largest: number | null; largestBasis: Basis | null } {
  const list = quantities.filter((q) => q.measure === measure);
  const top = list.reduce<(typeof list)[number] | null>((best, q) => (best === null || q.value > best.value ? q : best), null);
  const largest = top?.value ?? null;
  const largestBasis = top?.basis ?? null;
  if (list.length === 0) return { ok: false, reason: "no stated capacity", largest, largestBasis };
  const families = new Set(list.map((q) => basisFamily(q.basis)));
  if (families.has(null)) return { ok: false, reason: "a range cannot be added", largest, largestBasis };
  if (families.size > 1) return { ok: false, reason: "stated with different bases (about, up to, more than)", largest, largestBasis };
  return {
    ok: true,
    value: list.reduce((sum, q) => sum + q.value, 0),
    family: [...families][0] as BasisFamily,
    approximate: list.some((q) => q.basis === "approximately" || q.basis === "up_to_approximately"),
    count: list.length,
  };
}
