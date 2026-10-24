import Image from "next/image";
import type { OrgView } from "@/lib/capital/ledger";
import styles from "./capital.module.css";

/**
 * Decorative company mark. A vendored lab mark (LobeHub, MIT) where one exists for the
 * company itself, else the company's own square picture (X, LinkedIn or site icon),
 * else a text monogram. The name is always rendered next to it.
 */
export function OrgMark({ org, large = false }: { org: Pick<OrgView, "logo" | "avatar" | "monogram">; large?: boolean }) {
  const avatar = org.logo ? null : org.avatar;
  const className = [styles.mark, large ? styles.markLarge : "", avatar ? styles.markAvatar : ""].filter(Boolean).join(" ");
  const src = org.logo ?? avatar;
  const size = org.logo ? (large ? 28 : 16) : large ? 48 : 28;
  return (
    <span className={className} aria-hidden="true">
      {src ? <Image src={src} alt="" width={size} height={size} unoptimized /> : org.monogram}
    </span>
  );
}

/**
 * Mark plus linked name, for tables. With `rowLink`, the name is the primary link of a row
 * marked data-row-link, so the whole row opens the organization's page.
 */
export function OrgCell({ org, sub, rowLink = false }: { org: OrgView; sub?: React.ReactNode; rowLink?: boolean }) {
  const name = org.href ? (
    <a className={styles.cellMain} href={org.href} data-row-primary={rowLink ? "" : undefined}>
      {org.name}
    </a>
  ) : (
    <span className={styles.cellMain}>{org.name}</span>
  );
  return (
    <span className={styles.orgCell}>
      <OrgMark org={org} />
      <span>
        {name}
        {sub ? <span className={styles.cellSub}>{sub}</span> : null}
      </span>
    </span>
  );
}
