import type { AmountView } from "@/lib/capital/ledger";
import { DitherBar } from "@/components/explore/DitherViz";
import { InstantTooltip } from "@/components/ui/InstantTooltip";
import styles from "./capital.module.css";

/**
 * One typed amount: dense symbol form ("≈$11.6B") with the plain words in the tooltip and for
 * screen readers. Press-reported values carry their outlet; they are never totals.
 *
 * Numbers stay ink; emphasis is weight, not a second colour.
 */
export function Amount({ view, emphasis = false }: { view: AmountView; emphasis?: boolean }) {
  return (
    <InstantTooltip
      className={`${styles.num} ${view.amount.status === "reported" ? styles.reported : ""}`}
      style={emphasis ? { fontWeight: 500 } : undefined}
      content={`${view.words} · ${view.statusLabel}${view.term ? ` · ${view.term}` : ""}`}
      describe={false}
    >
      <span aria-hidden="true">{view.short}</span>
      <span className={styles.srOnly}>{view.words}</span>
    </InstantTooltip>
  );
}

/**
 * The dithered magnitude under an amount, scaled to the largest amount in its
 * own table (`value / max`). Decorative: the digits beside it carry the value,
 * and the bar is the same ordered-dither mark as every chart on the site.
 */
export function AmountBar({ value, max }: { value: number | null; max: number }) {
  if (value === null || !Number.isFinite(value) || value <= 0 || !(max > 0)) return null;
  return (
    <span className={styles.amountBar}>
      <DitherBar value={value / max} height={8} color="blue" variant="gradient" track />
    </span>
  );
}

/**
 * The scale for the dithered bars inside one table: the largest stated value in
 * each currency, so a bar never compares a euro figure with a dollar one.
 */
export function maxByCurrency(rows: ReadonlyArray<{ value: number | null; currency: string }>): Map<string, number> {
  const max = new Map<string, number>();
  for (const row of rows) {
    if (row.value === null || row.value <= 0) continue;
    max.set(row.currency, Math.max(max.get(row.currency) ?? 0, row.value));
  }
  return max;
}

/** "—" with a reason in the tooltip. Unknown is never shown as zero. */
export function Missing({ reason = "Not tracked yet" }: { reason?: string }) {
  return (
    <InstantTooltip className={styles.dash} content={reason} describe={false}>
      <span aria-hidden="true">—</span>
      <span className={styles.srOnly}>{reason}</span>
    </InstantTooltip>
  );
}
