"use client";

import { useRef } from "react";
import styled from "styled-components";
import type { DailyContribution, TooltipPosition, GraphColorPalette } from "@/lib/types";
import { formatCurrency, formatTokenCount } from "@/lib/utils";
import { formatContributionDate } from "@/lib/date-utils";

interface TooltipProps {
  day: DailyContribution | null;
  position: TooltipPosition | null;
  visible: boolean;
  palette: GraphColorPalette;
}

const TooltipContainer = styled.div`
  position: fixed;
  z-index: 50;
  pointer-events: none;
`;

const TooltipCard = styled.div`
  border-radius: 16px;
  box-shadow: 0 20px 25px -5px rgba(0, 0, 0, 0.1), 0 10px 10px -5px rgba(0, 0, 0, 0.04);
  border-width: 1px;
  border-style: solid;
  padding: 16px;
  min-width: 220px;
  backdrop-filter: blur(8px);
`;

const DateText = styled.div`
  font-weight: 700;
  font-size: 16px;
  margin-bottom: 12px;
`;

const Divider = styled.div`
  border-top-width: 1px;
  border-top-style: solid;
  margin-top: 12px;
  margin-bottom: 12px;
`;

const FlexRow = styled.div<{ $marginBottom?: number; $marginTop?: number }>`
  display: flex;
  justify-content: space-between;
  align-items: center;
  ${props => props.$marginBottom && `margin-bottom: ${props.$marginBottom}px;`}
  ${props => props.$marginTop && `margin-top: ${props.$marginTop}px;`}
`;

const Label = styled.span`
  font-size: 14px;
  font-weight: 500;
`;

const TotalValue = styled.span`
  font-weight: 700;
  font-size: 20px;
  letter-spacing: -0.025em;
`;

const TokenList = styled.div`
  display: flex;
  flex-direction: column;
  gap: 8px;
  font-size: 14px;
`;

const TokenLabel = styled.span`
  font-weight: 500;
`;

const TokenValue = styled.span`
  font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, "Liberation Mono", "Courier New", monospace;
  font-weight: 600;
`;

const CostLabel = styled.span`
  font-size: 14px;
  font-weight: 600;
`;

const CostValue = styled.span`
  font-weight: 700;
`;

const MessageLabel = styled.span`
  font-size: 14px;
  font-weight: 500;
`;

const MessageValue = styled.span`
  font-size: 14px;
  font-weight: 600;
`;

function useAdjustedPosition(
  position: TooltipPosition | null,
  visible: boolean,
  tooltipRef: React.RefObject<HTMLDivElement | null>
): TooltipPosition | null {
  if (!visible || !position) return null;

  const tooltip = tooltipRef.current;
  if (!tooltip) {
    return { x: position.x + 15, y: position.y + 15 };
  }

  const rect = tooltip.getBoundingClientRect();
  const viewportWidth = typeof window !== "undefined" ? window.innerWidth : 1920;
  const viewportHeight = typeof window !== "undefined" ? window.innerHeight : 1080;

  let x = position.x + 15;
  let y = position.y + 15;

  if (x + rect.width > viewportWidth - 10) {
    x = position.x - rect.width - 15;
  }
  if (y + rect.height > viewportHeight - 10) {
    y = position.y - rect.height - 15;
  }

  return { x: Math.max(10, x), y: Math.max(10, y) };
}

export function Tooltip({ day, position, visible, palette }: TooltipProps) {
  const tooltipRef = useRef<HTMLDivElement>(null);
  const adjustedPosition = useAdjustedPosition(position, visible, tooltipRef);

  if (!visible || !day || !adjustedPosition) return null;

  const { totals, tokenBreakdown } = day;

  return (
    <TooltipContainer
      ref={tooltipRef}
      role="tooltip"
      style={{ left: adjustedPosition.x, top: adjustedPosition.y }}
    >
      <TooltipCard
        style={{
          backgroundColor: "var(--color-card-bg)",
          borderColor: "var(--color-border-default)",
          color: "var(--color-fg-default)",
        }}
      >
        <DateText style={{ color: "var(--color-fg-default)" }}>
          {formatContributionDate(day)}
        </DateText>

        <Divider style={{ borderColor: "var(--color-border-default)" }} />

        <FlexRow $marginBottom={12}>
          <Label style={{ color: "var(--color-fg-muted)" }}>Total Tokens</Label>
          <TotalValue
            style={{
              color: day.intensity >= 3 ? palette.grade1 : day.intensity >= 2 ? palette.grade2 : "var(--color-fg-default)",
            }}
          >
            {formatTokenCount(totals.tokens)}
          </TotalValue>
        </FlexRow>

        <Divider style={{ borderColor: "var(--color-border-default)" }} />

        <TokenList>
          <TokenRow label="Input" value={tokenBreakdown.input} />
          <TokenRow label="Output" value={tokenBreakdown.output} />
          <TokenRow label="Cache Read" value={tokenBreakdown.cacheRead} />
          <TokenRow label="Cache Write" value={tokenBreakdown.cacheWrite} />
          {tokenBreakdown.reasoning > 0 && <TokenRow label="Reasoning" value={tokenBreakdown.reasoning} />}
        </TokenList>

        <Divider style={{ borderColor: "var(--color-border-default)" }} />

        <FlexRow>
          <CostLabel style={{ color: "var(--color-fg-muted)" }}>Cost</CostLabel>
          <CostValue style={{ color: "var(--color-fg-default)" }}>
            {formatCurrency(totals.cost)}
          </CostValue>
        </FlexRow>

        <FlexRow $marginTop={8}>
          <MessageLabel style={{ color: "var(--color-fg-muted)" }}>Messages</MessageLabel>
          <MessageValue style={{ color: "var(--color-fg-default)" }}>
            {totals.messages.toLocaleString()}
          </MessageValue>
        </FlexRow>
      </TooltipCard>
    </TooltipContainer>
  );
}

function TokenRow({ label, value }: { label: string; value: number }) {
  if (value === 0) return null;

  return (
    <FlexRow>
      <TokenLabel style={{ color: "var(--color-fg-muted)" }}>{label}</TokenLabel>
      <TokenValue style={{ color: "var(--color-fg-default)" }}>
        {formatTokenCount(value)}
      </TokenValue>
    </FlexRow>
  );
}
