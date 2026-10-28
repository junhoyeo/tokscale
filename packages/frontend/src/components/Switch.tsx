"use client";

import styled from "styled-components";
import { DitherSwitch } from "dither-kit/DitherSwitch";

interface SwitchProps {
  checked: boolean;
  onChange: (checked: boolean) => void;
  leftLabel: string;
  rightLabel: string;
  className?: string;
}

/**
 * Two named states around the kit's dithered switch. The labels are real
 * buttons, so either side can be chosen by pointer or keyboard.
 */
export function Switch({ checked, onChange, leftLabel, rightLabel, className }: SwitchProps) {
  return (
    <SwitchContainer className={className}>
      <Label type="button" $active={!checked} onClick={() => onChange(false)}>
        {leftLabel}
      </Label>
      <DitherSwitch
        value={checked}
        onChange={onChange}
        color="blue"
        label={`Sort by ${checked ? rightLabel : leftLabel}`}
      />
      <Label type="button" $active={checked} onClick={() => onChange(true)}>
        {rightLabel}
      </Label>
    </SwitchContainer>
  );
}

const SwitchContainer = styled.div`
  display: inline-flex;
  align-items: center;
  gap: 4px;
`;

const Label = styled.button<{ $active: boolean }>`
  padding: 6px 4px;
  color: ${({ $active }) => ($active ? "var(--v2-text)" : "var(--v2-text-muted)")};
  font-size: 14px;
  font-weight: ${({ $active }) => ($active ? 600 : 400)};
  text-decoration: none;

  &:hover {
    color: var(--v2-text);
  }

  &:focus-visible {
    outline: 2px solid var(--v2-focus);
    outline-offset: 2px;
  }
`;
