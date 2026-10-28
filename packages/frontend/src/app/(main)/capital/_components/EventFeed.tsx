import Link from "next/link";
import { formatPartialDate, type EventView } from "@/lib/capital/ledger";
import { Amount, Missing } from "./Amount";
import { SourceLink } from "./SourceLink";
import styles from "./capital.module.css";

/**
 * Newest-first list of events. The type is a text label, not a colour.
 *
 * `ledger` is the landing page's reading size (16px) with a header row, so the
 * feed reads as the ledger it is; the default is the app screens' dense row.
 */
export function EventFeed({
  events,
  showParties = true,
  variant = "compact",
}: {
  events: EventView[];
  showParties?: boolean;
  variant?: "compact" | "ledger";
}) {
  if (events.length === 0) return <p className={styles.empty}>No events tracked yet.</p>;
  return (
    <>
      {variant === "ledger" ? (
        <div className={styles.feedHead} aria-hidden="true">
          <span>Date</span>
          <span>Type</span>
          <span>Event</span>
          <span>Amount</span>
          <span>Status</span>
          <span>Source</span>
        </div>
      ) : null}
      <ol className={`${styles.feed} ${variant === "ledger" ? styles.feedLedger : ""}`}>
        {events.map((e) => {
          const headline = e.headline?.amount ?? null;
          return (
            <li className={styles.feedRow} key={e.id} data-row-link="">
              <span className={styles.feedDate}>{formatPartialDate(e.date)}</span>
              <span className={styles.tag}>{e.tag}</span>
              <span className={styles.feedTitle}>
                <Link href={e.href} data-row-primary="">
                  {e.title}
                </Link>
                {showParties ? <span className={styles.cellSub}>{e.partiesText}</span> : null}
              </span>
              <span className={styles.feedValue}>
                {headline ? (
                  <>
                    <Amount view={headline} />
                    {e.headline?.countedElsewhere ? <span className={styles.cellSub}>counted in a later event</span> : null}
                  </>
                ) : e.headline ? (
                  <span className={styles.num}>{e.headline.short}</span>
                ) : (
                  <Missing reason="No amount stated in the source" />
                )}
                {e.valuation ? (
                  <span className={styles.cellSub}>
                    at <Amount view={e.valuation} /> post
                  </span>
                ) : null}
              </span>
              <span className={styles.feedStatus}>
                <span className={styles.statusText}>{e.statusLabel}</span>
              </span>
              <span className={styles.feedSource}>
                <SourceLink source={e.headline?.source ?? e.sources[0]} />
              </span>
            </li>
          );
        })}
      </ol>
    </>
  );
}
