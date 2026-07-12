"use client";

import { useId, type CSSProperties } from "react";
import styled from "styled-components";
import { formatNumber } from "@/lib/utils";
import type { ProfileStatsData } from "./types";

export interface TokenBreakdownProps {
  stats: ProfileStatsData;
  className?: string;
}

const BreakdownPanel = styled.section`
  overflow: hidden;
  border: 1px solid var(--service-border);
  border-radius: 12px;
  background: var(--service-surface);
  color: var(--service-text);
`;

const BreakdownHeader = styled.header`
  padding: 0.875rem 1rem;
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

const SegmentedBar = styled.div`
  display: flex;
  width: 100%;
  height: 10px;
  overflow: hidden;
  border: 1px solid var(--service-border);
  border-radius: 4px;
  background: var(--service-surface-muted);
`;

const Segment = styled.span<{ $color: string; $weight: number }>`
  min-width: ${(props) => (props.$weight > 0 ? "3px" : 0)};
  flex: ${(props) => props.$weight} 1 0;
  background: ${(props) => props.$color};
`;

const BreakdownList = styled.dl`
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(7.5rem, 1fr));
  gap: 0.75rem;
  margin: 0.875rem 0 0;
`;

const BreakdownItem = styled.div`
  min-width: 0;
  border-left: 2px solid var(--segment-color);
  padding-left: 0.5rem;
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
  border-radius: 2px;
  box-shadow: inset 0 0 0 1px rgb(255 255 255 / 0.08);
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
  gap: 0.25rem;
  margin: 0.25rem 0 0;
  color: var(--service-text);
  font-size: 0.9375rem;
  font-variant-numeric: tabular-nums;
  font-weight: 600;
  line-height: 1.2;
`;

const Percentage = styled.span`
  color: var(--service-text-muted);
  font-size: 0.6875rem;
  font-weight: 500;
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
      color: "var(--service-accent)",
    },
    {
      label: "Output",
      value: finiteNonnegative(stats.outputTokens),
      color: "var(--service-accent-hover)",
    },
    {
      label: "Cache read",
      value: finiteNonnegative(stats.cacheReadTokens),
      color:
        "color-mix(in srgb, var(--service-accent) 55%, var(--service-text-muted))",
    },
    {
      label: "Cache write",
      value: finiteNonnegative(stats.cacheWriteTokens),
      color:
        "color-mix(in srgb, var(--service-accent) 30%, var(--service-text-muted))",
    },
    ...(finiteNonnegative(stats.reasoningTokens) > 0
      ? [
          {
            label: "Reasoning",
            value: finiteNonnegative(stats.reasoningTokens),
            color: "var(--service-text-muted)",
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
          Distribution across input, output, cache, and reasoning tokens.
        </BreakdownDescription>
      </BreakdownHeader>

      <BreakdownBody>
        <SegmentedBar
          role="img"
          aria-label={`Token distribution: ${describedBreakdown}`}
        >
          {tokenTypes
            .filter((type) => type.value > 0)
            .map((type) => (
              <Segment
                key={type.label}
                $color={type.color}
                $weight={type.value}
                aria-hidden="true"
                title={`${type.label}: ${formatNumber(type.value)}`}
              />
            ))}
        </SegmentedBar>

        <BreakdownList>
          {tokenTypes.map((type) => {
            const percentage =
              Number.isFinite(type.value) &&
              Number.isFinite(breakdownTotal) &&
              breakdownTotal > 0
                ? (type.value / breakdownTotal) * 100
                : 0;

            return (
              <BreakdownItem
                key={type.label}
                style={{ "--segment-color": type.color } as CSSProperties}
              >
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
