"use client";

import Link from "next/link";
import styled, { css } from "styled-components";

/** Shared v2 building blocks for the home and tracker pages. */

export const focusRing = css`
  &:focus-visible {
    outline: 2px solid var(--v2-focus);
    outline-offset: 3px;
  }
`;

export const PageShell = styled.div`
  isolation: isolate;
  color-scheme: dark;
  min-height: 100dvh;
  overflow-x: clip;
  background: var(--v2-canvas);
  color: var(--v2-text);
  font-variant-numeric: tabular-nums;
`;

export const Container = styled.div`
  width: 100%;
  max-width: 1200px;
  margin: 0 auto;
  padding: 0 32px;

  @media (max-width: 520px) {
    padding: 0 16px;
  }
`;

/* The shared eyebrow (Title Case, no letter-spacing). */
export { Eyebrow } from "@/components/ui/Eyebrow";

export const DisplayTitle = styled.h1`
  margin: 0;
  color: var(--v2-text);
  font-family: var(--v2-font-display);
  font-size: clamp(44px, 6.2vw, 84px);
  font-weight: 400;
  line-height: 1.02;
  letter-spacing: -0.012em;
  text-wrap: balance;
`;

export const SectionTitle = styled.h2`
  margin: 0;
  color: var(--v2-text);
  font-family: var(--v2-font-display);
  font-size: clamp(32px, 3.6vw, 38px);
  font-weight: 400;
  line-height: 1.08;
  letter-spacing: -0.008em;
  text-wrap: balance;
`;

export const Lead = styled.p`
  margin: 0;
  color: var(--v2-text-muted);
  font-size: 17px;
  line-height: 1.6;
  text-wrap: pretty;

  @media (max-width: 520px) {
    font-size: 16px;
  }
`;

const buttonBase = css`
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 8px;
  min-height: 44px;
  padding: 0 18px;
  border-radius: 8px;
  font-size: 15px;
  font-weight: 600;
  white-space: nowrap;
  text-decoration: none;
  transition: background-color 120ms ease, border-color 120ms ease, color 120ms ease;
  ${focusRing}

  @media (prefers-reduced-motion: reduce) {
    transition: none;
  }
`;

/* White on #0073FF is 4.3:1, just under AA for 15px text, so filled buttons
   use the slightly deeper --v2-accent-strong (5.2:1). */
export const PrimaryButton = styled(Link)`
  ${buttonBase}
  border: 1px solid var(--v2-accent-strong);
  background: var(--v2-accent-strong);
  color: #fff;

  &:hover {
    border-color: var(--v2-accent);
    background: var(--v2-accent);
  }
`;

export const SecondaryButton = styled(Link)`
  ${buttonBase}
  border: 1px solid var(--v2-rule-strong);
  background: transparent;
  color: var(--v2-text);

  &:hover {
    border-color: rgba(255, 255, 255, 0.34);
    background: rgba(255, 255, 255, 0.05);
  }
`;

export const TextLink = styled(Link)`
  display: inline-flex;
  align-items: center;
  gap: 6px;
  border-radius: 4px;
  color: var(--v2-text);
  font-size: 15px;
  font-weight: 600;
  text-decoration: none;
  ${focusRing}

  span[aria-hidden] {
    color: var(--v2-accent);
    transition: transform 120ms ease;
  }

  &:hover span[aria-hidden] {
    transform: translateX(2px);
  }

  @media (prefers-reduced-motion: reduce) {
    span[aria-hidden] {
      transition: none;
    }

    &:hover span[aria-hidden] {
      transform: none;
    }
  }
`;

export const Section = styled.section`
  position: relative;
  padding: 120px 0 0;

  @media (max-width: 720px) {
    padding-top: 80px;
  }
`;

export const SectionHeader = styled.header<{ $center?: boolean }>`
  display: flex;
  flex-direction: column;
  gap: 16px;
  max-width: 640px;
  ${({ $center }) =>
    $center &&
    css`
      align-items: center;
      margin: 0 auto;
      text-align: center;
    `}
`;

export const VisuallyHidden = styled.span`
  position: absolute;
  width: 1px;
  height: 1px;
  margin: -1px;
  padding: 0;
  overflow: hidden;
  clip: rect(0 0 0 0);
  white-space: nowrap;
  border: 0;
`;

export function Arrow() {
  return <span aria-hidden="true">→</span>;
}
