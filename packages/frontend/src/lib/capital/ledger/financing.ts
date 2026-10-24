import { EVENT_TYPE_LABELS, MEASURE_LABELS } from "./labels";
import type { AmountView, EventView } from "./model";
import type { CapitalEvent, EventType, Measure } from "./schema";

/** Financing is an economic category, independent of an event's canonical route. */
const FINANCING_TYPES: ReadonlySet<EventType> = new Set([
  "equity_round", "strategic_investment", "debt_facility", "tender_offer", "ipo", "registration", "form_d",
]);

export const EQUITY_MEASURES: readonly Measure[] = ["equity_raised", "equity_investment", "investment_commitment"];
export const FINANCING_MEASURES: readonly Measure[] = [
  "equity_raised", "ipo_proceeds", "debt_raised", "form_d_amount_sold",
  "equity_investment", "investment_commitment", "secondary_sale", "form_d_offering_amount",
];

export function isFinancing(event: CapitalEvent): boolean {
  return event.kind === "financing" || event.kind === "ipo" || event.kind === "filing" ||
    event.types.some((type) => FINANCING_TYPES.has(type)) ||
    event.amounts.some((amount) => FINANCING_MEASURES.includes(amount.measure));
}

/** Select a financing amount, never a compute contract. Included amounts stay labeled, not summed. */
export function financingAmount(
  event: EventView,
  measures: readonly Measure[] = FINANCING_MEASURES,
  recipient?: string,
): AmountView | null {
  const candidates = event.amounts.filter((a) => !recipient || !a.amount.to || a.amount.to === recipient);
  // Deduplication affects totals, not instrument priority: a current investment included in a
  // later round still comes before a conditional commitment to invest more in the future.
  for (const measure of measures) {
    if (!FINANCING_MEASURES.includes(measure)) continue;
    for (const skipped of [null, "included_in"] as const) {
      const amount = candidates.find((a) => a.amount.measure === measure && a.skipped === skipped);
      if (amount) return amount;
    }
  }
  return null;
}

/** Instrument labels for funding history, not the mixed event's first (possibly compute) type. */
export function financingLabel(event: EventView): string {
  if (event.event.roundLabel) return event.event.roundLabel;
  const labels = event.event.types.filter((type) => FINANCING_TYPES.has(type)).map((type) => EVENT_TYPE_LABELS[type]);
  const amount = financingAmount(event);
  return labels.join(", ") || (amount ? MEASURE_LABELS[amount.amount.measure] : "Financing");
}

/** A participant's own financing, not a loan or investment it made in someone else. */
export function isCompanyFinancing(event: CapitalEvent, orgId: string): boolean {
  return isFinancing(event) && event.participants.some((p) =>
    p.org === orgId && p.roles.some((role) => role === "issuer" || role === "filer" || role === "borrower"),
  );
}
