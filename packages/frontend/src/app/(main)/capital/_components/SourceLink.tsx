import { sourceShortLabel, type Source } from "@/lib/capital/ledger";
import { InstantTooltip } from "@/components/ui/InstantTooltip";
import styles from "./capital.module.css";

/** Short label for a source: the SEC form ("8-K"), else the publisher. */
export { sourceShortLabel };

/** External link to a source with a short label. The full title is in the tooltip. */
export function SourceLink({ source, label }: { source: Source; label?: string }) {
  return (
    <InstantTooltip
      as="a"
      className={styles.srcLink}
      href={source.url}
      target="_blank"
      rel="noopener noreferrer"
      content={source.title}
    >
      {label ?? sourceShortLabel(source)} ↗
    </InstantTooltip>
  );
}
