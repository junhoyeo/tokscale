"use client";

import Link from "next/link";
import styled, { css } from "styled-components";

/**
 * Shared building blocks for the home and tracker pages, on the dither-cell
 * motif: one text face (Figtree), one accent blue, square 2px corners, no
 * lines — surfaces separate by a step in tone, selections by a dithered
 * accent underline as wide as their label.
 */

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

/* One pair of margins for the whole page, on the same edges as the
   navigation bar (max-width 1500, padding 32 / 16 below 520). */
export const Container = styled.div`
  width: 100%;
  max-width: 1500px;
  margin: 0 auto;
  padding-inline: 32px;

  @media (max-width: 520px) {
    padding-inline: 16px;
  }
`;

export const DisplayTitle = styled.h1`
  margin: 0;
  color: var(--v2-text);
  font-size: clamp(34px, 4.4vw, 56px);
  font-weight: 500;
  line-height: 1.05;
  letter-spacing: -0.02em;
  text-wrap: balance;
`;

export const SectionTitle = styled.h2`
  margin: 0;
  color: var(--v2-text);
  font-size: clamp(28px, 2.9vw, 36px);
  font-weight: 500;
  line-height: 1.1;
  letter-spacing: -0.015em;
  text-wrap: balance;
`;

export const Lead = styled.p`
  margin: 0;
  max-width: 36rem;
  color: var(--v2-text-muted);
  font-size: 17px;
  line-height: 1.6;
  text-wrap: pretty;

  @media (max-width: 520px) {
    font-size: 16px;
  }
`;

export const TextLink = styled(Link)`
  display: inline-flex;
  align-items: center;
  gap: 6px;
  min-height: 32px;
  border-radius: 2px;
  color: var(--v2-text);
  font-size: 15px;
  font-weight: 500;
  text-decoration: underline;
  text-decoration-color: transparent;
  text-underline-offset: 0.18em;
  text-decoration-thickness: 0.08em;
  transition: text-decoration-color 160ms ease;
  ${focusRing}

  span[aria-hidden] {
    color: var(--v2-accent);
    transition: transform 160ms ease;
  }

  @media (hover: hover) {
    &:hover {
      text-decoration-color: currentColor;

      span[aria-hidden] {
        transform: translateX(2px);
      }
    }
  }

  @media (prefers-reduced-motion: reduce) {
    transition: none;

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
  padding-top: 128px;

  @media (max-width: 720px) {
    padding-top: 80px;
  }
`;

export const SectionHeader = styled.header`
  display: flex;
  flex-direction: column;
  gap: 16px;
  max-width: 640px;
`;

export const VisuallyHidden = styled.span`
  position: absolute;
  width: 1px;
  height: 1px; /* taste-skill-ignore: screen-reader-only text */
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
