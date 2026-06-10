"use client";

import styled, { css } from "styled-components";

/**
 * Shared primitives for the profile page's "list table" cards (Models,
 * Devices). Colors are applied inline by consumers via CSS variables; these
 * only own layout/typography so the two tables can't drift apart.
 *
 * Grid columns are intentionally part of the shared header/row because both
 * tables use the same `1fr auto auto` (+1 metric column at 480px) layout. If
 * a future table needs a different column count, override
 * `grid-template-columns` locally via `styled(ListHeader)` / `styled(ListRow)`.
 */

export const ListCard = styled.div`
  border-radius: 1rem;
  border-width: 1px;
  border-style: solid;
  overflow: hidden;
`;

export const ListHeader = styled.div`
  display: grid;
  grid-template-columns: 1fr auto auto;
  gap: 0.75rem;
  padding: 0.75rem;
  font-size: 0.75rem;
  font-weight: 500;
  text-transform: uppercase;
  letter-spacing: 0.05em;
  border-bottom-width: 1px;
  border-bottom-style: solid;

  @media (min-width: 480px) {
    grid-template-columns: 1fr auto auto auto;
    gap: 1rem;
    padding-left: 1rem;
    padding-right: 1rem;
  }

  @media (min-width: 640px) {
    padding-left: 1.5rem;
    padding-right: 1.5rem;
  }
`;

export const ListRow = styled.div`
  display: grid;
  grid-template-columns: 1fr auto auto;
  gap: 0.75rem;
  padding: 0.75rem;
  align-items: center;

  @media (min-width: 480px) {
    grid-template-columns: 1fr auto auto auto;
    gap: 1rem;
    padding-left: 1rem;
    padding-right: 1rem;
  }

  @media (min-width: 640px) {
    padding-left: 1.5rem;
    padding-right: 1.5rem;
  }
`;

export const ListMetricCell = styled.div<{
  $width: string;
  $smWidth: string;
  $hideOnMobile?: boolean;
}>`
  text-align: right;
  width: ${(props) => props.$width};

  ${(props) =>
    props.$hideOnMobile &&
    css`
      @media (max-width: 479px) {
        display: none;
      }
    `}

  @media (min-width: 640px) {
    width: ${(props) => props.$smWidth};
  }
`;
