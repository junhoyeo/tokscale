"use client";

import styled from "styled-components";
import { SOURCE_LOGOS } from "@/lib/constants";
import type { ClientType } from "@/lib/types";

interface SourceLogoProps {
  sourceId: string;
  height?: number;
  className?: string;
  /**
   * When the logo sits next to a visible text label of the same source, mark it
   * decorative so assistive tech doesn't announce the source name twice.
   */
  decorative?: boolean;
}

const StyledImg = styled.img<{ $height: number }>`
  border-radius: 2px;
  object-fit: contain;
  height: ${props => props.$height}px;
  width: auto;
  min-width: ${props => props.$height}px;
  max-width: ${props => props.$height}px;
  min-height: ${props => props.$height}px;
  max-height: ${props => props.$height}px;
`;

export function SourceLogo({ sourceId, height = 14, className = "", decorative = false }: SourceLogoProps) {
  const normalizedId = sourceId.toLowerCase() as ClientType;
  const src = Object.prototype.hasOwnProperty.call(SOURCE_LOGOS, normalizedId)
    ? SOURCE_LOGOS[normalizedId]
    : null;

  if (!src) {
    return (
      <span className={className} aria-hidden={decorative || undefined}>
        {sourceId}
      </span>
    );
  }

  return (
    <StyledImg
      src={src}
      alt={decorative ? "" : sourceId}
      $height={height}
      className={className}
    />
  );
}
