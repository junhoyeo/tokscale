import { formatMoney, formatPartialDate } from "@/lib/capital/ledger";
import { calendarSpanLabel, fiscalLabel, type CapexView } from "@/lib/capital/ledger/capex";
import { InstantTooltip } from "@/components/ui/InstantTooltip";
import { CapexBars } from "./CapexBars";
import { CapexSources } from "./CapexSources";
import styles from "./capital.module.css";

/** Quarterly cash capex for one company: bars plus the table with fiscal labels and filings. */
export function CapexTable({ view, name, limit = 8 }: { view: CapexView; name: string; limit?: number }) {
  const quarters = view.quarters.slice(-limit).reverse();
  const years = view.years.slice(-3).reverse();
  return (
    <div>
      <div style={{ maxWidth: 420, marginBottom: 16 }}>
        <CapexBars quarters={view.quarters} label={`${name} quarterly cash paid for property and equipment`} />
      </div>
      <table className={`${styles.ledgerTable} ${styles.ledgerStack}`}>
        <thead>
          <tr>
            <th scope="col">Quarter</th>
            <th scope="col">Period</th>
            <th scope="col" className={styles.right}>
              Cash paid for PP&amp;E
            </th>
            <th scope="col">How</th>
            <th scope="col">Filing</th>
          </tr>
        </thead>
        <tbody>
          {quarters.map((q) => (
            <tr key={`${q.start}-${q.end}`}>
              <td className={styles.full}>
                <span className={styles.cellMain}>{fiscalLabel(q)}</span>
              </td>
              <td data-label="Period">
                {calendarSpanLabel(q.start, q.end)}
                <span className={styles.cellSub}>
                  {formatPartialDate(q.start)} to {formatPartialDate(q.end)}
                </span>
              </td>
              <td className={`${styles.right} ${styles.num}`} data-label="Value">
                {formatMoney(q.value, q.currency)}
              </td>
              <td data-label="How">
                {q.derived ? (
                  <InstantTooltip content={q.formula} describe={false}>
                    Derived
                    <span className={styles.cellSub}>{q.formula}</span>
                  </InstantTooltip>
                ) : (
                  "Reported"
                )}
              </td>
              <td data-label="Filing">
                <CapexSources sources={q.sources} />
              </td>
            </tr>
          ))}
        </tbody>
      </table>
      {years.length > 0 ? (
        <p className={styles.note}>
          Fiscal years:{" "}
          {years.map((y, i) => (
            <span key={y.fiscalYear}>
              {i > 0 ? " · " : ""}FY{y.fiscalYear} {formatMoney(y.value, y.currency)} ({calendarSpanLabel(y.start, y.end)},{" "}
              <a className={styles.link} href={y.source.url} target="_blank" rel="noopener noreferrer">
                {y.source.form}
              </a>
              )
            </span>
          ))}
        </p>
      ) : null}
      {view.notes.length > 0 ? <p className={styles.note}>{view.notes.join(" ")}</p> : null}
    </div>
  );
}
