import { formatMoney } from "@/lib/capital/ledger";
import { calendarSpanLabel, fiscalLabel, type CapexQuarter } from "@/lib/capital/ledger/capex";
import { InstantTooltip } from "@/components/ui/InstantTooltip";
import styles from "./capital.module.css";

/**
 * Small bar chart of quarterly cash capex. Derived quarters (year-to-date minus the earlier
 * year-to-date) are hollow. Each bar has a hover tooltip; the table on the spending page holds
 * the same numbers.
 */
export function CapexBars({ quarters, label }: { quarters: CapexQuarter[]; label: string }) {
  const shown = quarters.slice(-8);
  if (shown.length === 0) return null;
  const max = Math.max(...shown.map((q) => q.value));
  const width = 160;
  const height = 56;
  const gap = 3;
  const barWidth = (width - gap * (shown.length - 1)) / shown.length;
  return (
    <svg className={styles.bars} viewBox={`0 0 ${width} ${height}`} preserveAspectRatio="none" role="img" aria-label={label}>
      {shown.map((q, i) => {
        const h = max > 0 ? Math.max(1, (q.value / max) * (height - 2)) : 1;
        return (
          <InstantTooltip
            as="rect"
            key={`${q.start}-${q.end}`}
            className={q.derived ? styles.barDerived : styles.barFill}
            x={i * (barWidth + gap)}
            y={height - h}
            width={barWidth}
            height={h}
            vectorEffect={q.derived ? "non-scaling-stroke" : undefined}
            content={`${fiscalLabel(q)} (${calendarSpanLabel(q.start, q.end)}): ${formatMoney(q.value, q.currency)}${q.derived ? " (derived)" : ""}`}
            describe={false}
          />
        );
      })}
    </svg>
  );
}
