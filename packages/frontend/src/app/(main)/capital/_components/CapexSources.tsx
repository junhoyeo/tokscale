import { formatPartialDate } from "@/lib/capital/ledger";
import type { CapexSourceRef } from "@/lib/capital/ledger/capex";
import styles from "./capital.module.css";

/** Every filing used for a quarter, including both operands of a derived value. */
export function CapexSources({ sources, compact = false }: { sources: readonly CapexSourceRef[]; compact?: boolean }) {
  return (
    <>
      {sources.map((source, index) => (
        <span key={source.accession} style={{ display: compact ? "inline" : "block" }}>
          {compact && index > 0 ? " · " : null}
          <a
            className={styles.srcLink}
            href={source.url}
            target="_blank"
            rel="noopener noreferrer"
            aria-label={`${source.form} filed ${formatPartialDate(source.filed)}, accession ${source.accession}`}
          >
            {source.form}{compact ? "" : ` ${formatPartialDate(source.filed)}`} ↗
          </a>
        </span>
      ))}
    </>
  );
}
