"use client";

import { useState } from "react";
import styled from "styled-components";
import type { TokenContributionData } from "@/lib/types";
import { DataInput } from "@/components/DataInput";
import { GraphContainer } from "@/components/GraphContainer";
import { Navigation } from "@/components/layout/Navigation";
import { PublicFooter } from "@/components/layout/PublicFooter";
import { SecondaryButton } from "@/components/leaderboard/RankingUI";

const dayFormatter = new Intl.DateTimeFormat("en-US", {
  day: "numeric",
  month: "short",
  timeZone: "UTC",
  year: "numeric",
});

const costFormatter = new Intl.NumberFormat("en-US", {
  currency: "USD",
  maximumFractionDigits: 2,
  minimumFractionDigits: 2,
  style: "currency",
});

/** "Nov 6, 2025" for a YYYY-MM-DD calendar day; anything else is shown as it came. */
function formatExportDay(value: string | undefined): string {
  const match = value ? /^(\d{4})-(\d{2})-(\d{2})$/.exec(value) : null;
  if (!match) return value ?? "";
  return dayFormatter.format(Date.UTC(Number(match[1]), Number(match[2]) - 1, Number(match[3])));
}

export function formatExportRange(range: { start?: string; end?: string } | undefined): string {
  const start = formatExportDay(range?.start);
  const end = formatExportDay(range?.end);
  if (!start || !end) return start || end || "No dates";
  return start === end ? start : `${start} – ${end}`;
}

export default function LocalClient() {
  const [data, setData] = useState<TokenContributionData | null>(null);

  return (
    <div className="service-page-shell">
      <Navigation />

      <main className="service-main" id="main-content">
        <Frame>
          <Header>
            <Title>Local viewer</Title>
            <Lead>
              Load an export from <code>tokscale graph</code> and inspect it in your browser. Nothing is uploaded.
            </Lead>
          </Header>

          {!data ? (
            <DataInput onDataLoaded={setData} />
          ) : (
            <Content>
              <LoadedBar>
                <Summary aria-label="Loaded export">
                  <SummaryItem>
                    <dt>Date range</dt>
                    <dd>{formatExportRange(data.meta?.dateRange)}</dd>
                  </SummaryItem>
                  <SummaryItem>
                    <dt>Total cost</dt>
                    <dd>{costFormatter.format(data.summary.totalCost)}</dd>
                  </SummaryItem>
                  <SummaryItem>
                    <dt>Active days</dt>
                    <dd>{data.summary.activeDays?.toLocaleString("en-US") ?? "—"}</dd>
                  </SummaryItem>
                </Summary>
                <ReloadButton type="button" onClick={() => setData(null)}>
                  Load different data
                </ReloadButton>
              </LoadedBar>
              <GraphContainer data={data} />
            </Content>
          )}
        </Frame>
      </main>

      <PublicFooter />
    </div>
  );
}

/* One frame for the header and the data, centred with equal margins. */
const Frame = styled.div`
  width: 100%;
  max-width: 1120px;
  margin: 0 auto;
`;

const Header = styled.header`
  max-width: 760px;
  margin-bottom: 32px;
`;

const Title = styled.h1`
  margin: 0;
  color: var(--service-text);
  font-size: 1.75rem;
  font-weight: 500;
  letter-spacing: -0.02em;
  line-height: 1.15;
`;

const Lead = styled.p`
  margin: 12px 0 0;
  color: var(--service-text-muted);
  font-size: 1rem;
  line-height: 1.6;

  code {
    color: var(--service-text);
    font-size: 0.875rem;
    white-space: nowrap;
  }
`;

const Content = styled.div`
  display: flex;
  min-width: 0;
  flex-direction: column;
  gap: 20px;
`;

const LoadedBar = styled.div`
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: space-between;
  gap: 12px 24px;
  min-width: 0;
`;

/* Ruled strip: one rule above and below, one between facts, the numbers in ink
   and tabular. */
const Summary = styled.dl`
  display: flex;
  flex-wrap: wrap;
  min-width: 0;
  margin: 0;
  border-top: 1px solid var(--service-border);
  border-bottom: 1px solid var(--service-border);

  @media (max-width: 640px) {
    display: grid;
    width: 100%;
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
`;

const SummaryItem = styled.div`
  min-width: 0;
  padding: 10px 24px;

  &:first-child {
    padding-left: 0;
  }

  & + & {
    border-left: 1px solid var(--service-border);
  }

  dt {
    color: var(--service-text-muted);
    font-size: 0.8125rem;
    font-weight: 500;
  }

  dd {
    margin: 2px 0 0;
    color: var(--service-text);
    font-size: 1rem;
    font-weight: 500;
    font-variant-numeric: tabular-nums;
  }

  @media (max-width: 640px) {
    padding: 10px 12px 10px 0;

    &:first-child {
      grid-column: 1 / -1;
      border-bottom: 1px solid var(--service-border);
    }

    & + & {
      border-left: 0;
    }

    &:last-child {
      padding-left: 12px;
      border-left: 1px solid var(--service-border);
    }
  }
`;

const ReloadButton = styled(SecondaryButton)`
  flex: 0 0 auto;
`;
