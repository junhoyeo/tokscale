"use client";

import { useSearchParams } from "next/navigation";
import styled from "styled-components";
import { SegmentedGroup, SegmentButton } from "@/components/leaderboard/RankingUI";
import Link from "next/link";
import {
  LEADERBOARD_VIEWS,
  LEADERBOARD_VIEW_LABELS,
  buildLeaderboardViewHref,
  type LeaderboardSearchParams,
  type LeaderboardView,
} from "./views";

// Title row (the view's name, one sentence, the view's one action), then the
// view tabs as links on a full-width rule: the URL is authoritative, so SSR
// and back/forward behave naturally and the link is shareable.
//
// Uses aria-current="page" rather than role="tablist": these are full-page
// navigations, not in-page tab panels, so the link semantics are the honest
// thing.

export { buildLeaderboardViewHref };
export type { LeaderboardSearchParams, LeaderboardView };

const Header = styled.header`
  margin-bottom: 24px;

  @media (max-width: 640px) {
    margin-bottom: 16px;
  }
`;

/* The view tabs sit on a full-width rule; the active tab's accent line is
 * exactly as wide as its label and replaces its slice of the rule. */
const ViewTabs = styled(SegmentedGroup)`
  display: flex;
  width: 100%;
  border-bottom: 2px solid var(--service-border);
`;

/* Title row: h1 + sentence on the left, the view's one action on the right. */
const TitleRow = styled.div`
  display: flex;
  align-items: flex-end;
  justify-content: space-between;
  gap: 16px;
  margin-bottom: 20px;

  @media (max-width: 640px) {
    flex-direction: column;
    align-items: stretch;
    gap: 12px;
    margin-bottom: 14px;
  }
`;

const HeadingGroup = styled.div`
  min-width: 0;
`;

const Title = styled.h1`
  margin: 0;
  color: var(--service-text);
  font-size: 1.75rem;
  font-weight: 500;
  letter-spacing: -0.02em;
  line-height: 1.15;
  text-wrap: balance;

  @media (max-width: 640px) {
    font-size: 1.5rem;
  }
`;

const Description = styled.p`
  max-width: 68ch;
  margin: 6px 0 0;
  color: var(--service-text-muted);
  font-size: 0.875rem;
  line-height: 1.5;
  text-wrap: pretty;
`;

const Action = styled.div`
  display: flex;
  flex: 0 0 auto;
  align-items: center;
`;

/* A view tab is a link with the segment's selection mark: the accent line
 * sits on the tab row's own bottom rule. */
const ViewTab = styled(SegmentButton).attrs({ as: Link })`
  --segment-pad-x: 2px;
`;

const VIEW_DESCRIPTIONS: Record<LeaderboardView, string> = {
  humans: "Public Tokscale profiles ranked by the usage they submitted.",
  models: "Models ranked by how many people used them, from public Tokscale submissions.",
  agents: "Coding agents ranked by how many people used them, from public Tokscale submissions.",
  groups: "Scoped rankings for teams, friends, and workspaces.",
};

const VIEW_TITLES: Record<LeaderboardView, string> = {
  humans: "Human leaderboard",
  models: "Model leaderboard",
  agents: "Agent leaderboard",
  groups: "Groups",
};

interface ViewSelectorProps {
  current: LeaderboardView;
  searchParams: LeaderboardSearchParams;
  /** The view's one primary action, rendered right of the title. */
  action?: React.ReactNode;
}

export default function ViewSelector({ current, searchParams, action }: ViewSelectorProps) {
  const liveSearchParams = useSearchParams();
  const liveSearchParamsRecord: LeaderboardSearchParams = {};

  liveSearchParams.forEach((value, key) => {
    const existing = liveSearchParamsRecord[key];
    if (existing === undefined) {
      liveSearchParamsRecord[key] = value;
    } else if (Array.isArray(existing)) {
      existing.push(value);
    } else {
      liveSearchParamsRecord[key] = [existing, value];
    }
  });

  const activeSearchParams = liveSearchParams.toString()
    ? liveSearchParamsRecord
    : searchParams;

  return (
    <Header>
      <TitleRow>
        <HeadingGroup>
          <Title>{VIEW_TITLES[current]}</Title>
          <Description>{VIEW_DESCRIPTIONS[current]}</Description>
        </HeadingGroup>
        {action && <Action>{action}</Action>}
      </TitleRow>
      <ViewTabs as="nav" aria-label="Leaderboard view">
        {LEADERBOARD_VIEWS.map((view) => (
          <ViewTab
            key={view}
            href={buildLeaderboardViewHref(activeSearchParams, view)}
            $active={current === view}
            aria-current={current === view ? "page" : undefined}
          >
            {LEADERBOARD_VIEW_LABELS[view]}
          </ViewTab>
        ))}
      </ViewTabs>
    </Header>
  );
}
