import type { ElementType, ReactNode } from "react";
import styles from "./Eyebrow.module.css";

interface EyebrowProps {
  children: ReactNode;
  /** Defaults to a paragraph. */
  as?: ElementType;
  id?: string;
  className?: string;
}

/**
 * The small label above a heading, a number or a group. Write the text in Title Case
 * (use titleCase() from "@/lib/text/titleCase" for text built from data); the label is
 * never uppercased or letter-spaced.
 */
export function Eyebrow({ children, as: Tag = "p", id, className }: EyebrowProps) {
  return (
    <Tag id={id} className={className ? `${styles.eyebrow} ${className}` : styles.eyebrow}>
      {children}
    </Tag>
  );
}
