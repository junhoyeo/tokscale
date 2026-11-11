import Link from "next/link";
import {
  capitalPath,
  companiesPath,
  computePath,
  investorsPath,
  methodologyPath,
  roundsPath,
  spendingPath,
} from "@/lib/capital";
import styles from "./capital.module.css";

export type CapitalSection =
  | "overview"
  | "companies"
  | "investors"
  | "rounds"
  | "compute"
  | "spending"
  | "methodology";

const ITEMS: Array<{ key: CapitalSection; label: string; href: string }> = [
  { key: "overview", label: "Overview", href: capitalPath() },
  { key: "companies", label: "Companies", href: companiesPath() },
  { key: "investors", label: "Investors", href: investorsPath() },
  { key: "rounds", label: "Rounds", href: roundsPath() },
  { key: "compute", label: "Compute", href: computePath() },
  { key: "spending", label: "Spending", href: spendingPath() },
  { key: "methodology", label: "Method", href: methodologyPath() },
];

/** One thin row of section links under the main navigation. */
export function SubNav({ active }: { active?: CapitalSection }) {
  return (
    <nav className={styles.subnav} aria-label="Capital">
      <ul className={styles.subnavList}>
        {ITEMS.map((item) => (
          <li key={item.key}>
            <Link href={item.href} aria-current={item.key === active ? "page" : undefined}>
              {item.label}
            </Link>
          </li>
        ))}
      </ul>
    </nav>
  );
}
