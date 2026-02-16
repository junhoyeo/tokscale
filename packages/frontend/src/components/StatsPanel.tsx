"use client";

import styled from "styled-components";
import type { TokenContributionData, GraphColorPalette } from "@/lib/types";
import {
  formatCurrency,
  formatTokenCount,
  calculateCurrentStreak,
  calculateLongestStreak,
  findBestDay,
} from "@/lib/utils";
import { formatContributionDate } from "@/lib/date-utils";

interface StatsPanelProps {
  data: TokenContributionData;
  palette: GraphColorPalette;
}

const Container = styled.div`
  border-radius: 16px;
  border: 1px solid;
  padding: 24px;
  box-shadow: 0 1px 2px 0 rgb(0 0 0 / 0.05);
  transition: box-shadow 0.15s ease-in-out;
  background-color: var(--color-card-bg);
  border-color: var(--color-border-default);

  &:hover {
    box-shadow: 0 4px 6px -1px rgb(0 0 0 / 0.1), 0 2px 4px -2px rgb(0 0 0 / 0.1);
  }
`;

const Heading = styled.h3`
  font-size: 14px;
  font-weight: 700;
  margin-bottom: 16px;
  text-transform: uppercase;
  letter-spacing: 0.05em;
  color: var(--color-fg-muted);
`;

const Grid = styled.div`
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 24px;

  @media (max-width: 560px) {
    gap: 16px;
  }

  @media (max-width: 400px) {
    grid-template-columns: minmax(0, 1fr);
  }

  @media (min-width: 640px) {
    grid-template-columns: repeat(3, minmax(0, 1fr));
  }

  @media (min-width: 768px) {
    grid-template-columns: repeat(4, minmax(0, 1fr));
  }
`;

const SourcesContainer = styled.div`
  margin-top: 24px;
  padding-top: 24px;
  border-top: 1px solid;
  border-color: var(--color-border-default);
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  align-items: center;
`;

const SourcesLabel = styled.span`
  font-size: 12px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.05em;
  margin-right: 12px;
  color: var(--color-fg-muted);

  @media (max-width: 480px) {
    width: 100%;
    margin-right: 0;
  }
`;

const SourceBadge = styled.span<{ $backgroundColor: string }>`
  font-size: 12px;
  padding: 6px 12px;
  border-radius: 9999px;
  font-weight: 500;
  transition: all 200ms ease-in-out;
  background-color: ${props => props.$backgroundColor};
  color: var(--color-fg-default);
  max-width: 100%;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;

  &:hover {
    transform: scale(1.05);
  }
`;

const StatItemContainer = styled.div`
  display: flex;
  flex-direction: column;
  gap: 4px;
  min-width: 0;
`;

const StatItemLabel = styled.div`
  font-size: 12px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.05em;
  color: var(--color-fg-muted);
  overflow-wrap: anywhere;
`;

const StatItemValue = styled.div<{ $highlight?: boolean; $color?: string }>`
  font-weight: 700;
  letter-spacing: -0.025em;
  font-size: ${props => props.$highlight ? '20px' : '18px'};
  color: ${props => props.$color || 'var(--color-fg-default)'};
  min-width: 0;
  overflow-wrap: anywhere;

  @media (max-width: 400px) {
    font-size: ${props => props.$highlight ? '18px' : '16px'};
  }
`;

const StatItemSubValue = styled.div`
  font-size: 12px;
  font-weight: 500;
  color: var(--color-fg-muted);
`;

export function StatsPanel({ data, palette }: StatsPanelProps) {
  const { summary, contributions } = data;
  const currentStreak = calculateCurrentStreak(contributions);
  const longestStreak = calculateLongestStreak(contributions);
  const bestDay = findBestDay(contributions);

  return (
    <Container>
      <Heading>Statistics</Heading>

      <Grid>
        <StatItem label="Total Cost" value={formatCurrency(summary.totalCost)} highlightColor={palette.grade1} highlight />
        <StatItem label="Total Tokens" value={formatTokenCount(summary.totalTokens)} />
        <StatItem label="Active Days" value={`${summary.activeDays} / ${summary.totalDays}`} />
        <StatItem label="Avg / Day" value={formatCurrency(summary.averagePerDay)} />
        <StatItem label="Current Streak" value={`${currentStreak} day${currentStreak !== 1 ? "s" : ""}`} />
        <StatItem label="Longest Streak" value={`${longestStreak} day${longestStreak !== 1 ? "s" : ""}`} />
        {bestDay && bestDay.totals.cost > 0 && (
          <StatItem label="Best Day" value={formatContributionDate(bestDay)} subValue={formatCurrency(bestDay.totals.cost)} />
        )}
        <StatItem label="Models" value={summary.models.length.toString()} />
      </Grid>

      <SourcesContainer>
        <SourcesLabel>Sources:</SourcesLabel>
        {summary.sources.map((source) => (
          <SourceBadge
            key={source}
            $backgroundColor={`${palette.grade3}20`}
          >
            {source}
          </SourceBadge>
        ))}
      </SourcesContainer>
    </Container>
  );
}

interface StatItemProps {
  label: string;
  value: string;
  subValue?: string;
  highlightColor?: string;
  highlight?: boolean;
}

function StatItem({ label, value, subValue, highlightColor, highlight }: StatItemProps) {
  return (
    <StatItemContainer>
      <StatItemLabel>{label}</StatItemLabel>
      <StatItemValue
        $highlight={highlight}
        $color={highlight && highlightColor ? highlightColor : undefined}
      >
        {value}
      </StatItemValue>
      {subValue && <StatItemSubValue>{subValue}</StatItemSubValue>}
    </StatItemContainer>
  );
}
