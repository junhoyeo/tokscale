import { basisSymbol, tryQuantityTotal } from "@/lib/capital/ledger";
import { Missing } from "./Amount";
import styles from "./capital.module.css";

type QuantityTotal = ReturnType<typeof tryQuantityTotal>;

/**
 * A capacity sum, or the largest stated value with its basis when the values cannot be added
 * ("up to 5 GW" and "about 3.5 GW" are never added together).
 */
export function CapacityTotal({ result, unit }: { result: QuantityTotal; unit: string }) {
  if (result.ok) {
    const prefix = result.family === "floor" ? ">" : result.family === "ceiling" ? "≤" : result.approximate ? "≈" : "";
    return (
      <>
        <span className={styles.num}>
          {prefix}
          {result.value.toLocaleString("en-US")} {unit}
        </span>
        {result.count > 1 ? <span className={styles.cellSub}>sum of {result.count} stated values</span> : null}
      </>
    );
  }
  if (result.largest === null) return <Missing reason="No capacity stated" />;
  return (
    <>
      <span className={styles.num}>
        largest {result.largestBasis ? basisSymbol(result.largestBasis) : ""}
        {result.largest.toLocaleString("en-US")} {unit}
      </span>
      <span className={styles.cellSub}>not added: {result.reason}</span>
    </>
  );
}
