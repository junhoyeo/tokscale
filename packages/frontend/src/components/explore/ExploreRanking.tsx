"use client";

import styled from "styled-components";
import type { AgentUsageRow, Coverage, ExplorePeriod, ExploreSort, ModelUsageRow } from "@/lib/explore";
import { ControlBar, CoverageLine, InlineLink, LinkSegments, type LinkOption } from "./ExploreUI";
import { AgentBoard, ModelBoard } from "./UsageBoards";

const CoverageRow = styled(CoverageLine)`
  margin-bottom: 12px;
`;

const After = styled.p`
  margin: 16px 0 0;
  color: var(--service-text-muted);
  font-size: 0.8125rem;
  line-height: 1.5;

  @media (max-width: 640px) {
    font-size: 0.875rem;
  }
`;

type ExploreRankingProps = {
  coverage: Coverage;
  period: ExplorePeriod;
  sort: ExploreSort;
  periodOptions: LinkOption[];
  sortOptions: LinkOption[];
  /** True when the rows were cut at the page size. */
  truncated: boolean;
} & ({ kind: "models"; rows: ModelUsageRow[] } | { kind: "agents"; rows: AgentUsageRow[] });

/** Period + sort controls, the coverage line and the ranking for one explore view. */
export function ExploreRanking(props: ExploreRankingProps) {
  const { coverage, period, sort, periodOptions, sortOptions, truncated } = props;
  return (
    <>
      <ControlBar>
        <LinkSegments label="Period" visibleLabel="Period" options={periodOptions} />
        <LinkSegments label="Sort by" visibleLabel="Sort" options={sortOptions} />
      </ControlBar>
      <CoverageRow coverage={coverage} />
      {props.kind === "models" ? (
        <ModelBoard rows={props.rows} coverage={coverage} period={period} sort={sort} />
      ) : (
        <AgentBoard rows={props.rows} coverage={coverage} period={period} sort={sort} />
      )}
      {props.kind === "models" ? (
        <After>
          {truncated ? `Showing the top ${props.rows.length}. ` : null}
          <InlineLink href="/models">Browse every model by lab</InlineLink>
        </After>
      ) : truncated ? (
        <After>Showing the top {props.rows.length}.</After>
      ) : null}
    </>
  );
}
