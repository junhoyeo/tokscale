"use client";

import Link from "next/link";
import styled from "styled-components";
import { DitherButton } from "dither-kit/DitherButton";

/**
 * The one primary action, as a dither-kit button that navigates. DitherButton
 * is built on Base UI's Button: `nativeButton={false}` plus
 * `render={<Link/>}` turns it into a real Next.js link with the button's
 * dithered canvas behind the label. The kit's tailwind utilities are unlayered,
 * so overrides are doubled (`&&`) to win regardless of insertion order.
 */
export const DitherButtonLink = styled(DitherButton)`
  && {
    min-height: 48px;
    padding: 0 22px;
    border-radius: var(--radius, 2px);
    background-color: rgba(0, 102, 230, 0.55);
    color: #fff;
    font-family: var(--v2-font-ui);
    font-size: 16px;
    font-weight: 600;
    letter-spacing: -0.005em;
    white-space: nowrap;
  }
`;

/** Props: DitherButton's plus the link's destination. */
export type DitherButtonLinkProps = React.ComponentProps<typeof DitherButton> & { href: string };

export function PrimaryAction({ href, children, ...rest }: DitherButtonLinkProps) {
  return (
    <DitherButtonLink nativeButton={false} render={<Link href={href} />} color="blue" variant="solid" {...rest}>
      {children}
    </DitherButtonLink>
  );
}
