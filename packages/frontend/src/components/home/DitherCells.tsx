"use client";

import styled from "styled-components";

/**
 * The motif at data scale: a field of square dither cells whose lit count
 * follows a 4×4 Bayer matrix, so density (not opacity or length) carries the
 * value — denser means more. Rendered as DOM cells so it is server-rendered,
 * crisp at any DPI, and every cell can answer a hover.
 */

const BAYER4 = [
  [0, 8, 2, 10],
  [12, 4, 14, 6],
  [3, 11, 1, 9],
  [15, 7, 13, 5],
].map((row) => row.map((v) => (v + 0.5) / 16));

export interface DitherCellsProps {
  /** The value to show. */
  value: number;
  /** The value that fills the field solid. */
  max: number;
  /** Cells across (columns). */
  cols?: number;
  /** Cells down (rows). */
  rows?: number;
  /** Cell size in px. */
  cell?: number;
  /** Fill colour of a lit cell; defaults to the data blue. */
  color?: string;
  /** Accessible name for the field; defaults to how full it is. */
  label?: string;
  className?: string;
}

export function DitherCells({ value, max, cols = 24, rows = 3, cell = 7, color, label, className }: DitherCellsProps) {
  const frac = max > 0 ? Math.min(1, Math.max(0, value / max)) : 0;
  const cells: boolean[] = [];
  for (let y = 0; y < rows; y++) {
    for (let x = 0; x < cols; x++) {
      cells.push(frac > BAYER4[y % 4][x % 4]);
    }
  }
  return (
    <Field
      className={className}
      role="img"
      aria-label={label ?? `Dithered cell field, ${Math.round(frac * 100)} percent filled`}
      $cols={cols}
      $rows={rows}
      $cell={cell}
      $color={color}
    >
      {cells.map((lit, i) => (
        <Cell key={i} $lit={lit} />
      ))}
    </Field>
  );
}

/**
 * The motif as a full-width band: a fixed row of equal cells whose tone
 * follows the weights (one per segment) through the same Bayer matrix, so a
 * heavier segment is denser but the band always spans the whole measure.
 */
export function DitherBand({
  weights,
  cells = 64,
  color,
  label,
  className,
}: {
  weights: number[];
  cells?: number;
  color?: string;
  label?: string;
  className?: string;
}) {
  const total = weights.reduce((sum, weight) => sum + Math.max(0, weight), 0);
  const max = Math.max(...weights, 0);
  const normalized = weights.map((weight) => (total > 0 ? Math.max(0, weight) / total : 0));
  const bounds = normalized.reduce<{ from: number; to: number }[]>((list, share) => {
    const from = list.length ? list[list.length - 1].to : 0;
    return [...list, { from, to: from + share }];
  }, []);
  const lit = Array.from({ length: cells }, (_, i) => {
    const p = (i + 0.5) / cells;
    const index = bounds.findIndex((bound) => p < bound.to);
    const weight = max > 0 ? Math.max(0, weights[index < 0 ? weights.length - 1 : index]) / max : 0;
    return weight > BAYER4[i % 4][0];
  });
  return (
    <Band className={className} role={label ? "img" : undefined} aria-label={label} aria-hidden={label ? undefined : "true"} $color={color}>
      {lit.map((on, i) => (
        <BandCell key={i} $lit={on} />
      ))}
    </Band>
  );
}

const Band = styled.span<{ $color?: string }>`
  display: grid;
  grid-auto-flow: column;
  grid-auto-columns: minmax(0, 1fr);
  gap: 2px;
  width: 100%;
  height: 26px;

  --cells-lit: ${({ $color }) => $color ?? "var(--v2-accent)"};
`;

const BandCell = styled.i<{ $lit: boolean }>`
  display: block;
  width: 100%;
  height: 100%;
  background: ${({ $lit }) => ($lit ? "var(--cells-lit)" : "#16335e")};
`;

const Field = styled.span<{ $cols: number; $rows: number; $cell: number; $color?: string }>`
  display: grid;
  grid-template-columns: repeat(${({ $cols }) => $cols}, ${({ $cell }) => $cell}px);
  grid-auto-rows: ${({ $cell }) => $cell}px;
  gap: 2px;
  width: fit-content;

  /* The unlit cells stay as a faint ground so the field's full extent reads. */
  --cells-lit: ${({ $color }) => $color ?? "var(--v2-accent)"};
`;

const Cell = styled.i<{ $lit: boolean }>`
  display: block;
  width: 100%;
  height: 100%;
  background: ${({ $lit }) => ($lit ? "var(--cells-lit)" : "var(--v2-surface-raised)")};
`;
