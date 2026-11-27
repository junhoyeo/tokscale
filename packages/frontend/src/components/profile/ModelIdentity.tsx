"use client";

import styled, { css } from "styled-components";
import { getModelColor } from "./modelColors";

export interface ModelIdentityProps {
  model: string;
  wrapOnMobile?: boolean;
  emphasize?: boolean;
  className?: string;
}

const IdentityRoot = styled.span`
  display: flex;
  min-width: 0;
  align-items: center;
  gap: 0.5rem;
`;

const IdentityMarker = styled.span`
  width: 8px;
  height: 8px;
  flex: 0 0 auto;
  border-radius: 50%;
`;

const IdentityName = styled.span<{
  $wrapOnMobile: boolean;
  $emphasize: boolean;
}>`
  overflow: hidden;
  color: var(--service-text);
  text-overflow: ellipsis;
  white-space: nowrap;

  ${(props) =>
    props.$emphasize &&
    css`
      font-weight: 500;
    `}

  ${(props) =>
    props.$wrapOnMobile &&
    css`
      @media (max-width: 639px) {
        overflow-wrap: anywhere;
        white-space: normal;
      }
    `}
`;

export function ModelIdentity({
  model,
  wrapOnMobile = false,
  emphasize = false,
  className,
}: ModelIdentityProps) {
  return (
    <IdentityRoot className={className}>
      <IdentityMarker
        aria-hidden="true"
        style={{ backgroundColor: getModelColor(model) }}
      />
      <IdentityName $wrapOnMobile={wrapOnMobile} $emphasize={emphasize}>
        {model}
      </IdentityName>
    </IdentityRoot>
  );
}
