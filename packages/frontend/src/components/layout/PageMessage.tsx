"use client";

import type { ReactNode } from "react";
import Link from "next/link";
import styled from "styled-components";
import { DitherButton } from "dither-kit/DitherButton";

type PrimaryAction = { label: string; href: string } | { label: string; onClick: () => void };

/**
 * A dead end on the site shell: one h1 that says what happened, one sentence,
 * one primary way out (a solid kit button), and at most one quiet text link.
 * Used by not-found and error.
 */
export function PageMessage({
  title,
  children,
  primary,
  secondary,
}: {
  title: string;
  children?: ReactNode;
  primary: PrimaryAction;
  secondary?: { label: string; href: string };
}) {
  return (
    <Message>
      <h1>{title}</h1>
      {children ? <Lead>{children}</Lead> : null}
      <Actions>
        {"href" in primary ? (
          <PrimaryButton
            color="blue"
            variant="solid"
            nativeButton={false}
            role="link"
            render={<Link href={primary.href} />}
          >
            {primary.label}
          </PrimaryButton>
        ) : (
          <PrimaryButton color="blue" variant="solid" onClick={primary.onClick}>
            {primary.label}
          </PrimaryButton>
        )}
        {secondary ? <TextLink href={secondary.href}>{secondary.label}</TextLink> : null}
      </Actions>
    </Message>
  );
}

const Message = styled.div`
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  padding: 48px 0 80px;

  h1 {
    margin: 0;
    color: var(--v2-text);
    /* The page's one statement and its focal point, at the display step:
       28px on a phone, about 42px at 768, 72px (5vw) at 1440. */
    font-size: clamp(1.75rem, 0.5rem + 4.4vw, 4.5rem);
    font-weight: 500;
    letter-spacing: -0.02em;
    line-height: 1.05;
    text-wrap: balance;
  }
`;

const Lead = styled.p`
  max-width: 56ch;
  margin: 24px 0 0;
  color: var(--v2-text);
  font-size: 18px;
  line-height: 1.6;

  a {
    color: inherit;
    text-decoration: underline;
    text-decoration-color: var(--v2-accent);
    text-decoration-thickness: 1px;
    text-underline-offset: 3px;
  }

  a:hover {
    text-decoration-color: currentColor;
  }

  a:focus-visible {
    outline: 2px solid var(--v2-focus);
    outline-offset: 3px;
  }
`;

const Actions = styled.div`
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 16px 28px;
  margin-top: 36px;
`;

/* Solid fill, white label. The element's own background is the same blue as
   the canvas fill, so the button reads as filled before the canvas paints. */
const PrimaryButton = styled(DitherButton)`
  min-height: 44px;
  background-color: var(--v2-accent-strong);
  color: #ffffff;
  font-family: var(--v2-font-ui);
  font-size: 15px;
  font-weight: 500;

  &:focus-visible {
    outline: 2px solid var(--v2-focus);
    outline-offset: 2px;
  }
`;

const TextLink = styled(Link)`
  display: inline-flex;
  align-items: center;
  min-height: 44px;
  color: var(--v2-text);
  font-size: 16px;
  text-decoration: underline;
  text-decoration-color: var(--v2-accent);
  text-decoration-thickness: 1px;
  text-underline-offset: 4px;

  &:hover {
    text-decoration-color: currentColor;
  }

  &:focus-visible {
    outline: 2px solid var(--v2-focus);
    outline-offset: 3px;
  }
`;
