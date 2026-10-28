"use client";

import styled from "styled-components";
import type { ModelRef } from "@/lib/explore";

/* The mark sits on the ground at its own size: no chip, no tint, no box. */
const ModelTile = styled.span<{ $size: number }>`
  display: inline-grid;
  width: ${({ $size }) => $size}px;
  height: ${({ $size }) => $size}px;
  flex: 0 0 auto;
  place-items: center;
  color: var(--service-text-muted);
  font-size: ${({ $size }) => Math.max(11, Math.round($size * 0.5))}px;
  font-weight: 600;
  line-height: 1;
`;

const ModelTileImage = styled.img<{ $size: number }>`
  display: block;
  width: ${({ $size }) => $size}px;
  height: ${({ $size }) => $size}px;
`;

/**
 * Decorative model family mark on a small tile; the model name is always
 * rendered next to it. Unknown families get the first letter of the name.
 */
export function ModelMark({ model, size = 18 }: { model: Pick<ModelRef, "name" | "logoUrl">; size?: number }) {
  const inner = Math.round(size * 0.62);
  return (
    <ModelTile $size={size} aria-hidden="true">
      {model.logoUrl ? (
        <ModelTileImage src={model.logoUrl} alt="" width={inner} height={inner} $size={inner} loading="lazy" />
      ) : (
        model.name.trim().charAt(0).toUpperCase() || "?"
      )}
    </ModelTile>
  );
}
