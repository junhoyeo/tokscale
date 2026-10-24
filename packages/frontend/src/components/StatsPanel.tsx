"use client";

import styled from "styled-components";
import type { TokenContributionData } from "@/lib/types";
import {
  formatCurrency,
  formatTokenCount,
  calculateCurrentStreak,
  calculateLongestStreak,
  findBestDay,
  dayHasActivity,
} from "@/lib/utils";
import { formatContributionDate } from "@/lib/date-utils";
import { formatDuration } from "@/lib/format";

interface StatsPanelProps {
  data: TokenContributionData;
  totalActiveTimeMs?: number | null;
  sessionCount?: number | null;
  mcpServers?: string[];
}

const Container = styled.section`
  min-width: 0;
  overflow: hidden;
  border: 1px solid var(--service-border);
  border-radius: 12px;
  background: var(--service-surface);
`;

const Header = styled.header`
  padding: 14px 20px;
  border-bottom: 1px solid var(--service-border);

  @media (max-width: 520px) {
    padding: 12px 14px;
  }
`;

const Heading = styled.h3`
  margin: 0;
  color: var(--service-text);
  font-size: 1rem;
  font-weight: 600;
  letter-spacing: -0.01em;
`;

const Body = styled.div`
  padding: 4px 20px 20px;

  @media (max-width: 520px) {
    padding: 2px 14px 14px;
  }
`;

/* Each fact sits on its own thin rule, so the grid reads as a ledger at every column count. */
const Grid = styled.dl`
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 0 24px;
  margin: 0;

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

const StatItemContainer = styled.div`
  display: flex;
  min-width: 0;
  flex-direction: column;
  gap: 4px;
  padding: 14px 0;
  border-bottom: 1px solid var(--service-border);
`;

const StatItemLabel = styled.dt`
  color: var(--service-text-muted);
  font-size: 0.8125rem;
  font-weight: 500;
  overflow-wrap: anywhere;
`;

const StatItemValue = styled.dd<{ $highlight?: boolean }>`
  min-width: 0;
  margin: 0;
  color: ${({ $highlight }) => ($highlight ? "var(--v2-numeric)" : "var(--service-text)")};
  font-size: ${({ $highlight }) => ($highlight ? "1.25rem" : "1.125rem")};
  font-weight: 600;
  font-variant-numeric: tabular-nums;
  letter-spacing: -0.02em;
  overflow-wrap: anywhere;

  @media (max-width: 400px) {
    font-size: ${({ $highlight }) => ($highlight ? "1.125rem" : "1rem")};
  }
`;

const StatItemSubValue = styled.dd`
  margin: 0;
  color: var(--service-text-muted);
  font-size: 0.8125rem;
  font-variant-numeric: tabular-nums;
`;

const BadgeRow = styled.div`
  display: grid;
  grid-template-columns: 96px minmax(0, 1fr);
  align-items: start;
  gap: 8px 12px;
  margin-top: 16px;

  @media (max-width: 480px) {
    grid-template-columns: minmax(0, 1fr);
  }
`;

const BadgeLabel = styled.span`
  padding-top: 4px;
  color: var(--service-text-muted);
  font-size: 0.8125rem;
  font-weight: 500;
`;

const Badges = styled.ul`
  display: flex;
  min-width: 0;
  flex-wrap: wrap;
  gap: 6px;
  margin: 0;
  padding: 0;
  list-style: none;
`;

const Badge = styled.li`
  max-width: 100%;
  min-width: 0;
  overflow: hidden;
  padding: 3px 10px;
  border: 1px solid var(--service-border);
  border-radius: 999px;
  background: var(--service-surface-muted);
  color: var(--service-text);
  font-size: 0.8125rem;
  text-overflow: ellipsis;
  white-space: nowrap;
`;

function BadgeList({ label, items }: { label: string; items: string[] }) {
  return (
    <BadgeRow>
      <BadgeLabel>{label}</BadgeLabel>
      <Badges aria-label={label}>
        {items.map((item) => (
          <Badge key={item} title={item}>
            {item}
          </Badge>
        ))}
      </Badges>
    </BadgeRow>
  );
}

export function StatsPanel({ data, totalActiveTimeMs, sessionCount, mcpServers }: StatsPanelProps) {
  const { summary, contributions } = data;
  const currentStreak = calculateCurrentStreak(contributions);
  const longestStreak = calculateLongestStreak(contributions);
  const bestDay = findBestDay(contributions);

  return (
    <Container aria-label="Statistics">
      <Header>
        <Heading>Statistics</Heading>
      </Header>

      <Body>
        <Grid>
          <StatItem label="Total cost" value={formatCurrency(summary.totalCost)} highlight />
          <StatItem label="Total tokens" value={formatTokenCount(summary.totalTokens)} />
          <StatItem label="Active days" value={`${summary.activeDays} / ${summary.totalDays}`} />
          <StatItem label="Avg / day" value={formatCurrency(summary.averagePerDay)} />
          <StatItem label="Current streak" value={`${currentStreak} day${currentStreak !== 1 ? "s" : ""}`} />
          <StatItem label="Longest streak" value={`${longestStreak} day${longestStreak !== 1 ? "s" : ""}`} />
          {bestDay && dayHasActivity(bestDay) && (
            <StatItem label="Best day" value={formatContributionDate(bestDay)} subValue={formatCurrency(bestDay.totals.cost)} />
          )}
          <StatItem label="Models" value={summary.models.length.toString()} />
          {totalActiveTimeMs != null && totalActiveTimeMs > 0 && (
            <StatItem label="Active time" value={formatDuration(totalActiveTimeMs)} />
          )}
          {sessionCount != null && sessionCount > 0 && (
            <StatItem label="Sessions" value={sessionCount.toString()} />
          )}
        </Grid>

        <BadgeList label="Clients" items={summary.clients} />
        {mcpServers && mcpServers.length > 0 && <BadgeList label="MCPs" items={mcpServers} />}
      </Body>
    </Container>
  );
}

interface StatItemProps {
  label: string;
  value: string;
  subValue?: string;
  highlight?: boolean;
}

function StatItem({ label, value, subValue, highlight }: StatItemProps) {
  return (
    <StatItemContainer>
      <StatItemLabel>{label}</StatItemLabel>
      <StatItemValue $highlight={highlight}>{value}</StatItemValue>
      {subValue && <StatItemSubValue>{subValue}</StatItemSubValue>}
    </StatItemContainer>
  );
}
