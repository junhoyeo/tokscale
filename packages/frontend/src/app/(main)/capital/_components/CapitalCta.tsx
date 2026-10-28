"use client";

import Link from "next/link";
import { DitherButton } from "dither-kit/DitherButton";
import styles from "./capital.module.css";

/**
 * The capital section's one primary action, rendered as a Next link. The label
 * sits on the fill, so the fill is solid blue: a hatched or dotted fill leaves
 * the black ground showing through the letters.
 */
export function CapitalCta({ href, children }: { href: string; children: React.ReactNode }) {
  return (
    <DitherButton
      color="blue"
      variant="solid"
      nativeButton={false}
      role="link"
      render={<Link href={href} />}
      className={styles.heroCta}
      style={{ minHeight: 44, paddingInline: 20 }}
    >
      {children}
    </DitherButton>
  );
}
