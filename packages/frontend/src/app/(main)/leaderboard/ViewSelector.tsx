"use client";

import Link from "next/link";
import { useSearchParams } from "next/navigation";
import styled from "styled-components";
import { SegmentButton, SegmentedGroup } from "@/components/leaderboard/RankingUI";

// Top-of-page segmented control that swaps between the global user leaderboard
// and the group browser. Pure-link nav (no client state), so SSR + back/forward
// behave naturally and the URL is shareable.
//
// Uses aria-current="page" rather than role="tablist": these are full-page
// navigations, not in-page tab panels, so the link semantics are the honest
// thing and keep ArrowLeft/Right doing whatever the browser would normally do
// for in-page focus.

export type LeaderboardView = "users" | "groups";

const Header = styled.header`
  display: flex;
  align-items: flex-end;
  justify-content: space-between;
  gap: 20px;
  margin-bottom: 24px;

  @media (max-width: 640px) {
    align-items: flex-start;
    flex-direction: column;
    gap: 16px;
  }
`;

const HeadingGroup = styled.div`
  min-width: 0;
`;

const Title = styled.h1`
  margin: 0;
  color: var(--service-text);
  font-size: 1.5rem;
  font-weight: 600;
  letter-spacing: -0.02em;
  text-wrap: balance;
`;

const Description = styled.p`
  max-width: 68ch;
  margin: 6px 0 0;
  color: var(--service-text-muted);
  font-size: 0.875rem;
  line-height: 1.5;
  text-wrap: pretty;

  @media (max-width: 640px) {
    font-size: 1rem;
  }
`;

export type LeaderboardSearchParams = Record<string, string | string[] | undefined>;

interface ViewSelectorProps {
  current: LeaderboardView;
  searchParams: LeaderboardSearchParams;
}

export function buildLeaderboardViewHref(
  searchParams: LeaderboardSearchParams,
  view: LeaderboardView
): string {
  const params = new URLSearchParams();
  const currentPeriod = typeof searchParams.period === "string" ? searchParams.period : undefined;

  for (const [key, value] of Object.entries(searchParams)) {
    if (key === "page" || key === "view" || value === undefined) {
      continue;
    }

    // Only carry from/to when the current period is "custom"; otherwise they
    // would prime stale date inputs on the destination view.
    if ((key === "from" || key === "to") && currentPeriod !== "custom") {
      continue;
    }

    if (Array.isArray(value)) {
      for (const item of value) {
        params.append(key, item);
      }
    } else {
      params.set(key, value);
    }
  }

  params.set("view", view);
  return `/leaderboard?${params.toString()}`;
}

export default function ViewSelector({ current, searchParams }: ViewSelectorProps) {
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
      <HeadingGroup>
        <Title>{current === "groups" ? "Groups" : "Leaderboard"}</Title>
        <Description>
          {current === "groups"
            ? "Scoped rankings for teams, friends, and workspaces."
            : "Global rankings across public Tokscale submissions."}
        </Description>
      </HeadingGroup>
      <SegmentedGroup as="nav" aria-label="Leaderboard view">
        <SegmentButton
          as={Link}
          href={buildLeaderboardViewHref(activeSearchParams, "users")}
          $active={current === "users"}
          aria-current={current === "users" ? "page" : undefined}
        >
          Users
        </SegmentButton>
        <SegmentButton
          as={Link}
          href={buildLeaderboardViewHref(activeSearchParams, "groups")}
          $active={current === "groups"}
          aria-current={current === "groups" ? "page" : undefined}
        >
          Groups
        </SegmentButton>
      </SegmentedGroup>
    </Header>
  );
}
