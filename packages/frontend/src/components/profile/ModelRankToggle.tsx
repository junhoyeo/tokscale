"use client";

import styled, { css } from "styled-components";
import type { ModelRankBy } from "./modelRanking";

export interface ModelRankToggleProps {
  value: ModelRankBy;
  onChange: (next: ModelRankBy) => void;
  ariaLabel: string;
  label?: string;
  className?: string;
}

const RANK_OPTIONS: ReadonlyArray<{ value: ModelRankBy; label: string }> = [
  { value: "cost", label: "Cost" },
  { value: "tokens", label: "Tokens" },
];

const Wrapper = styled.div`
  display: inline-flex;
  flex: 0 0 auto;
  align-items: center;
  gap: 0.5rem;
`;

const Label = styled.span`
  color: var(--service-text-muted);
  font-size: 0.75rem;
  font-weight: 500;
  white-space: nowrap;
`;

const SegmentedGroup = styled.div`
  display: inline-flex;
  align-items: center;
  padding: 0.125rem;
  background: var(--service-surface-muted);
  border: 1px solid var(--service-border);
  border-radius: 0.5rem;
`;

const SegmentedButton = styled.button<{ $active: boolean }>`
  position: relative;
  min-height: 1.75rem;
  padding: 0.25rem 0.625rem;
  color: var(--service-text-muted);
  background: transparent;
  border: 0;
  border-radius: 0.375rem;
  font: inherit;
  font-size: 0.75rem;
  font-weight: 500;
  line-height: 1;
  white-space: nowrap;
  cursor: pointer;
  transition:
    color 140ms ease,
    background-color 140ms ease;

  ${(props) =>
    props.$active &&
    css`
      color: var(--service-accent);
      background: var(--service-accent-soft);
    `}

  @media (hover: hover) {
    &:hover {
      color: ${(props) =>
        props.$active ? "var(--service-accent)" : "var(--service-text)"};
    }
  }

  &:focus-visible {
    outline: 2px solid var(--service-focus);
    outline-offset: 1px;
  }

  @media (pointer: coarse) {
    min-height: 2.75rem;
  }

  @media (prefers-reduced-motion: reduce) {
    transition: none;
  }
`;

export function ModelRankToggle({
  value,
  onChange,
  ariaLabel,
  label,
  className,
}: ModelRankToggleProps) {
  return (
    <Wrapper className={className}>
      {label && <Label aria-hidden="true">{label}</Label>}
      <SegmentedGroup role="group" aria-label={ariaLabel}>
        {RANK_OPTIONS.map((option) => {
          const isActive = value === option.value;
          return (
            <SegmentedButton
              key={option.value}
              type="button"
              $active={isActive}
              aria-pressed={isActive}
              onClick={() => onChange(option.value)}
            >
              {option.label}
            </SegmentedButton>
          );
        })}
      </SegmentedGroup>
    </Wrapper>
  );
}
