"use client";

import { useId } from "react";
import styled from "styled-components";
import { formatNumber } from "@/lib/utils";
import { DitherSegmentBar } from "./ditherControls";
import type { ProfileStatsData } from "./types";

export interface TokenBreakdownProps {
  stats: ProfileStatsData;
  className?: string;
}

const TOKEN_MIX_COLORS = {
  input: "#56b4e9",
  output: "#009e73",
  cacheRead: "#cc79a7",
  cacheWrite: "#e69f00",
  reasoning: "#d7dde8",
} as const;

const BreakdownPanel = styled.section`
  overflow: hidden;
  border: 1px solid var(--service-border);
  border-radius: var(--radius);
  background: var(--service-surface);
  color: var(--service-text);
  container-type: inline-size;
`;

const BreakdownHeader = styled.header`
  padding: 0.75rem 1rem;
  border-bottom: 1px solid var(--service-border);
`;

const BreakdownHeading = styled.h2`
  margin: 0;
  color: var(--service-text);
  font-size: 0.9375rem;
  font-weight: 600;
  letter-spacing: -0.01em;
`;

const BreakdownDescription = styled.p`
  max-width: 46ch;
  margin: 0.25rem 0 0;
  color: var(--service-text-muted);
  font-size: 0.8125rem;
  line-height: 1.45;
`;

const BreakdownBody = styled.div`
  padding: 0.875rem 1rem 1rem;
`;

const BarFrame = styled.div`
  position: relative;
  display: block;
  width: 100%;
`;

const BreakdownList = styled.dl`
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(7.5rem, 1fr));
  gap: 0.75rem 1rem;
  margin: 0.875rem 0 0;

  @container (max-width: 34rem) {
    grid-template-columns: repeat(2, minmax(0, 1fr));

    > div:last-child:nth-child(odd) {
      grid-column: 1 / -1;
    }
  }
`;

const BreakdownItem = styled.div`
  min-width: 0;
`;

const LabelRow = styled.div`
  display: flex;
  min-width: 0;
  align-items: center;
  gap: 0.375rem;
`;

const Marker = styled.span<{ $color: string }>`
  flex: 0 0 auto;
  width: 0.5rem;
  height: 0.5rem;
  background: ${(props) => props.$color};
  border-radius: 1px;
`;

const Label = styled.dt`
  overflow: hidden;
  color: var(--service-text-muted);
  font-size: 0.75rem;
  line-height: 1.2;
  text-overflow: ellipsis;
  white-space: nowrap;
`;

const ValueRow = styled.dd`
  display: flex;
  flex-wrap: wrap;
  align-items: baseline;
  gap: 0.375rem;
  margin: 0.25rem 0 0;
  color: var(--service-text);
  font-size: 0.9375rem;
  font-variant-numeric: tabular-nums;
  font-weight: 600;
  line-height: 1.2;
`;

const Percentage = styled.span`
  color: var(--service-text-muted);
  font-size: 0.75rem;
  font-weight: 400;
`;

function finiteNonnegative(value: number | undefined): number {
  return Number.isFinite(value) ? Math.max(0, value ?? 0) : 0;
}

export function TokenBreakdown({ stats, className }: TokenBreakdownProps) {
  const headingId = useId();
  const descriptionId = useId();
  const tokenTypes = [
    {
      label: "Input",
      value: finiteNonnegative(stats.inputTokens),
      color: TOKEN_MIX_COLORS.input,
    },
    {
      label: "Output",
      value: finiteNonnegative(stats.outputTokens),
      color: TOKEN_MIX_COLORS.output,
    },
    {
      label: "Cache read",
      value: finiteNonnegative(stats.cacheReadTokens),
      color: TOKEN_MIX_COLORS.cacheRead,
    },
    {
      label: "Cache write",
      value: finiteNonnegative(stats.cacheWriteTokens),
      color: TOKEN_MIX_COLORS.cacheWrite,
    },
    ...(finiteNonnegative(stats.reasoningTokens) > 0
      ? [
          {
            label: "Reasoning",
            value: finiteNonnegative(stats.reasoningTokens),
            color: TOKEN_MIX_COLORS.reasoning,
          },
        ]
      : []),
  ];
  const breakdownTotal = tokenTypes.reduce(
    (sum, type) => Math.min(Number.MAX_VALUE, sum + type.value),
    0,
  );
  const describedBreakdown = tokenTypes
    .map((type) => `${type.label} ${formatNumber(type.value)}`)
    .join(", ");

  return (
    <BreakdownPanel
      className={className}
      aria-labelledby={headingId}
      aria-describedby={descriptionId}
    >
      <BreakdownHeader>
        <BreakdownHeading id={headingId}>Token mix</BreakdownHeading>
        <BreakdownDescription id={descriptionId}>
          Share of input, output, cache, and reasoning tokens.
        </BreakdownDescription>
      </BreakdownHeader>

      <BreakdownBody>
        <BarFrame role="img" aria-label={`Token distribution: ${describedBreakdown}`}>
          <DitherSegmentBar
            height={12}
            segments={tokenTypes.map((type) => ({
              weight: type.value,
              color: type.color,
            }))}
          />
        </BarFrame>

        <BreakdownList>
          {tokenTypes.map((type) => {
            const percentage =
              Number.isFinite(type.value) &&
              Number.isFinite(breakdownTotal) &&
              breakdownTotal > 0
                ? (type.value / breakdownTotal) * 100
                : 0;

            return (
              <BreakdownItem key={type.label}>
                <LabelRow>
                  <Marker $color={type.color} aria-hidden="true" />
                  <Label>{type.label}</Label>
                </LabelRow>
                <ValueRow>
                  {formatNumber(type.value)}
                  <Percentage>{percentage.toFixed(1)}%</Percentage>
                </ValueRow>
              </BreakdownItem>
            );
          })}
        </BreakdownList>
      </BreakdownBody>
    </BreakdownPanel>
  );
}
