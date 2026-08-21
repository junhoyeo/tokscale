"use client";

import styled, { css } from "styled-components";

export const ListCard = styled.div`
  overflow: hidden;
  border-top: 1px solid var(--service-border);
  border-bottom: 1px solid var(--service-border);
  background: transparent;
  color: var(--service-text);
`;

export const ListTable = styled.table`
  width: 100%;
  border-collapse: collapse;
  table-layout: fixed;
  font-size: 0.8125rem;
  font-variant-numeric: tabular-nums;

  @media (max-width: 639px) {
    display: block;
  }
`;

export const ListCaption = styled.caption`
  position: absolute;
  width: 1px;
  height: 1px;
  padding: 0;
  margin: -1px;
  overflow: hidden;
  clip: rect(0, 0, 0, 0);
  white-space: nowrap;
  border: 0;
`;

export const ListHead = styled.thead`
  border-bottom: 1px solid var(--service-border);
  background: transparent;

  @media (max-width: 639px) {
    position: absolute;
    width: 1px;
    height: 1px;
    padding: 0;
    margin: -1px;
    overflow: hidden;
    clip: rect(0, 0, 0, 0);
    white-space: nowrap;
    border: 0;
  }
`;

export const ListBody = styled.tbody`
  @media (max-width: 639px) {
    display: block;
  }
`;

export const ListRow = styled.tr`
  border-top: 1px solid var(--service-border);

  &:first-child {
    border-top: 0;
  }

  @media (max-width: 639px) {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 0.625rem 1rem;
    padding: 0.75rem;
  }

  @media (min-width: 390px) and (max-width: 639px) {
    grid-template-columns: repeat(3, minmax(0, 1fr));
  }
`;

const tableCell = css<{
  $align?: "left" | "right";
  $width?: string;
}>`
  width: ${(props) => props.$width ?? "auto"};
  padding: 0.625rem 0.75rem;
  text-align: ${(props) => props.$align ?? "left"};
  vertical-align: middle;

  @media (min-width: 768px) {
    padding-right: 1rem;
    padding-left: 1rem;
  }
`;

export const ListHeaderCell = styled.th<{
  $align?: "left" | "right";
  $width?: string;
}>`
  ${tableCell}
  color: var(--service-text-muted);
  font-size: 0.75rem;
  font-weight: 500;
  white-space: nowrap;
`;

export const ListPrimaryCell = styled.th`
  padding: 0.625rem 0.75rem;
  color: var(--service-text);
  font-weight: 500;
  text-align: left;
  vertical-align: middle;

  @media (min-width: 768px) {
    padding-right: 1rem;
    padding-left: 1rem;
  }

  @media (max-width: 639px) {
    grid-column: 1 / -1;
    display: block;
    min-width: 0;
    padding: 0 0 0.125rem;
  }
`;

export const ListCell = styled.td<{
  $align?: "left" | "right";
  $width?: string;
}>`
  ${tableCell}
  color: var(--service-text);

  @media (max-width: 639px) {
    display: flex;
    min-width: 0;
    flex-direction: column;
    gap: 0.125rem;
    padding: 0;
    text-align: left;

    &::before {
      color: var(--service-text-muted);
      content: attr(data-label);
      font-size: 0.6875rem;
      font-weight: 500;
      line-height: 1.2;
    }
  }
`;

export const NumericValue = styled.span<{ $accent?: boolean }>`
  color: var(--service-text);
  font-variant-numeric: tabular-nums;

  ${(props) =>
    props.$accent &&
    css`
      font-weight: 600;
    `}
`;
