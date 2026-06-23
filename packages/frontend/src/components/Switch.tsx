"use client";

import styled from "styled-components";

interface SwitchProps {
  checked: boolean;
  onChange: (checked: boolean) => void;
  leftLabel: string;
  rightLabel: string;
  className?: string;
}

const SwitchContainer = styled.div`
  display: inline-flex;
  align-items: center;
  gap: 8px;
`;

const Label = styled.span<{ $active: boolean }>`
  font-size: 12px;
  font-weight: ${({ $active }) => ($active ? 600 : 400)};
  color: ${({ $active }) => ($active ? '#0073FF' : 'var(--color-fg-muted)')};
  transition: color 200ms, font-weight 200ms;
  cursor: pointer;
  user-select: none;
`;

const Track = styled.button<{ $checked: boolean }>`
  position: relative;
  width: 44px;
  height: 24px;
  border-radius: 12px;
  border: none;
  cursor: pointer;
  background-color: ${({ $checked }) => ($checked ? '#0073FF' : 'var(--color-border-default)')};
  transition: background-color 200ms;
  padding: 0;
  
  &:focus-visible {
    outline: none;
    box-shadow: 0 0 0 2px var(--color-bg-default), 0 0 0 4px #0073FF;
  }
`;

const Thumb = styled.span<{ $checked: boolean }>`
  position: absolute;
  top: 2px;
  left: ${({ $checked }) => ($checked ? '22px' : '2px')};
  width: 20px;
  height: 20px;
  border-radius: 50%;
  background-color: #fff;
  transition: left 200ms;
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.2);
`;

export function Switch({ checked, onChange, leftLabel, rightLabel, className }: SwitchProps) {
  return (
    <SwitchContainer className={className}>
      <Label 
        $active={!checked} 
        onClick={() => onChange(false)}
        aria-hidden="true"
      >
        {leftLabel}
      </Label>
      <Track
        type="button"
        role="switch"
        aria-checked={checked}
        aria-label={`Sort by ${checked ? rightLabel : leftLabel}`}
        $checked={checked}
        onClick={() => onChange(!checked)}
      >
        <Thumb $checked={checked} />
      </Track>
      <Label 
        $active={checked} 
        onClick={() => onChange(true)}
        aria-hidden="true"
      >
        {rightLabel}
      </Label>
    </SwitchContainer>
  );
}
