import type { Measure } from "./schema";

/**
 * The order in which an event's amounts are shown on its page, in feeds and on its card.
 *
 * Money that actually moved comes first: equity raised, IPO proceeds, the amount sold under a
 * Form D, debt. Then contract values and investments, then levels such as valuations. Figures that
 * are only ceilings or context come last: a Form D's total offering size (the amount the company
 * may sell, not what it sold), cumulative "has invested to date" statements and per-share prices.
 */
export const MEASURE_DISPLAY_ORDER: readonly Measure[] = [
  "equity_raised",
  "ipo_proceeds",
  "form_d_amount_sold",
  "debt_raised",
  "contract_value",
  "compute_commitment",
  "purchase_price",
  "equity_investment",
  "investment_commitment",
  "secondary_sale",
  "valuation_post",
  "valuation_pre",
  "revenue_reported",
  "revenue_run_rate",
  "capex_actual",
  "capex_guidance",
  "fund_mark",
  "estimated_capital_cost",
  "cumulative_investment",
  "form_d_offering_amount",
  "price_per_share",
];

export function displayRank(measure: Measure): number {
  const index = MEASURE_DISPLAY_ORDER.indexOf(measure);
  return index === -1 ? MEASURE_DISPLAY_ORDER.length : index;
}

/** Stable sort by display rank: items with the same measure keep their order in the source file. */
export function sortForDisplay<T>(items: readonly T[], measureOf: (item: T) => Measure): T[] {
  return items
    .map((item, index) => ({ item, index }))
    .sort((a, b) => displayRank(measureOf(a.item)) - displayRank(measureOf(b.item)) || a.index - b.index)
    .map(({ item }) => item);
}
