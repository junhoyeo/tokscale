"use client";

import { useLayoutEffect, useRef } from "react";
import styled from "styled-components";
import type { DailyContribution, TooltipPosition } from "@/lib/types";
import { formatCurrency, formatTokenCount } from "@/lib/utils";
import { formatContributionDate } from "@/lib/date-utils";

interface TooltipProps {
  day: DailyContribution | null;
  position: TooltipPosition | null;
  visible: boolean;
}

const TooltipContainer = styled.div`
  position: fixed;
  z-index: 50;
  pointer-events: none;
`;

const TooltipCard = styled.div`
  display: grid;
  gap: 10px;
  width: max-content;
  min-width: 220px;
  max-width: min(280px, calc(100vw - 20px));
  padding: 12px;
  border: 1px solid var(--service-border-strong);
  border-radius: 10px;
  background: var(--service-surface-muted);
  box-shadow: 0 12px 32px rgb(0 0 0 / 0.45);
  color: var(--service-text);
  font-size: 13px;
  font-variant-numeric: tabular-nums;
`;

const DateText = styled.div`
  color: var(--service-text);
  font-size: 13px;
  font-weight: 600;
`;

const Divider = styled.div`
  height: 1px;
  background: var(--service-border);
`;

const Row = styled.div`
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 16px;
`;

const Label = styled.span`
  color: var(--service-text-muted);
`;

const TotalValue = styled.span`
  color: var(--v2-numeric);
  font-size: 18px;
  font-weight: 600;
  letter-spacing: -0.02em;
`;

const Value = styled.span`
  color: var(--service-text);
  font-weight: 600;
`;

const List = styled.div`
  display: grid;
  gap: 6px;
`;

/**
 * Where the card goes: 15px below and right of the point, flipped to the other side when it would cross
 * the window edge, and never closer than 10px to the top or left.
 */
export function placeTooltip(
  position: TooltipPosition,
  size: { width: number; height: number },
  viewportWidth: number,
  viewportHeight: number,
): { left: number; top: number } {
  let x = position.x + 15;
  let y = position.y + 15;

  if (x + size.width > viewportWidth - 10) {
    x = position.x - size.width - 15;
  }
  if (y + size.height > viewportHeight - 10) {
    y = position.y - size.height - 15;
  }

  return { left: Math.max(10, x), top: Math.max(10, y) };
}

export function Tooltip({ day, position, visible }: TooltipProps) {
  const tooltipRef = useRef<HTMLDivElement>(null);

  // Measure after each render, before paint, so the card stays inside the window from its first frame.
  // (It used to read the previous frame's size, so a tooltip near an edge spilled over until the next move.)
  useLayoutEffect(() => {
    const tooltip = tooltipRef.current;
    if (!tooltip || !visible || !position) return;
    const { left, top } = placeTooltip(position, tooltip.getBoundingClientRect(), window.innerWidth, window.innerHeight);
    tooltip.style.left = `${left}px`;
    tooltip.style.top = `${top}px`;
  });

  if (!visible || !day || !position) return null;

  const { totals, tokenBreakdown } = day;

  return (
    <TooltipContainer
      ref={tooltipRef}
      role="tooltip"
      style={{ left: position.x + 15, top: position.y + 15 }}
    >
      <TooltipCard>
        <DateText>{formatContributionDate(day)}</DateText>

        <Row>
          <Label>Total tokens</Label>
          <TotalValue>{formatTokenCount(totals.tokens)}</TotalValue>
        </Row>

        <Divider />

        <List>
          <TokenRow label="Input" value={tokenBreakdown.input} />
          <TokenRow label="Output" value={tokenBreakdown.output} />
          <TokenRow label="Cache read" value={tokenBreakdown.cacheRead} />
          <TokenRow label="Cache write" value={tokenBreakdown.cacheWrite} />
          {tokenBreakdown.reasoning > 0 && <TokenRow label="Reasoning" value={tokenBreakdown.reasoning} />}
        </List>

        <Divider />

        <List>
          <Row>
            <Label>Cost</Label>
            <Value>{formatCurrency(totals.cost)}</Value>
          </Row>
          <Row>
            <Label>Messages</Label>
            <Value>{totals.messages.toLocaleString()}</Value>
          </Row>
        </List>
      </TooltipCard>
    </TooltipContainer>
  );
}

function TokenRow({ label, value }: { label: string; value: number }) {
  if (value === 0) return null;

  return (
    <Row>
      <Label>{label}</Label>
      <Value>{formatTokenCount(value)}</Value>
    </Row>
  );
}
