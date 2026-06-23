"use client";

import Link from "next/link";
import { useEffect, useRef, useState } from "react";
import styled from "styled-components";

// Top-of-page segmented control that swaps between the global user leaderboard
// and the group browser. Pure-link nav (no client state), so SSR + back/forward
// behave naturally and the URL is shareable.
//
// Uses aria-current="page" rather than role="tablist": these are full-page
// navigations, not in-page tab panels, so the link semantics are the honest
// thing and keep ArrowLeft/Right doing whatever the browser would normally do
// for in-page focus.

export type LeaderboardView = "users" | "groups";

const Bar = styled.nav`
  margin: 24px 0 0;
  display: flex;
  align-items: center;
  gap: 16px;
  flex-wrap: wrap;

  @media (max-width: 480px) {
    gap: 12px;
  }
`;

const Group = styled.div`
  display: inline-flex;
  padding: 4px;
  border: 1px solid var(--color-border-default);
  border-radius: 8px;
  background: var(--color-bg-subtle);
`;

const Item = styled(Link)<{ $active: boolean }>`
  display: inline-flex;
  align-items: center;
  gap: 6px;
  min-height: 32px;
  padding: 0 14px;
  border-radius: 6px;
  font-size: 13px;
  font-weight: 600;
  text-decoration: none;
  color: ${({ $active }) => ($active ? "var(--color-fg-default)" : "var(--color-fg-muted)")};
  background: ${({ $active }) => ($active ? "var(--color-bg-default)" : "transparent")};
  transition: background 0.12s, color 0.12s;

  &:hover {
    color: var(--color-fg-default);
  }

  &:focus-visible {
    outline: 2px solid var(--color-primary);
    outline-offset: 2px;
  }
`;

const Title = styled.h1`
  margin: 0;
  font-size: 30px;
  font-weight: 700;
  color: var(--color-fg-default);

  @media (max-width: 480px) {
    font-size: 24px;
  }
`;

const VisuallyHidden = styled.span`
  position: absolute;
  width: 1px;
  height: 1px;
  padding: 0;
  margin: -1px;
  overflow: hidden;
  clip: rect(0, 0, 0, 0);
  white-space: nowrap;
  border: 0;
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

const VIEW_LABELS: Record<LeaderboardView, string> = {
  users: "Users",
  groups: "Groups",
};

export default function ViewSelector({ current, searchParams }: ViewSelectorProps) {
  const [announcement, setAnnouncement] = useState("");
  const isFirstRender = useRef(true);

  useEffect(() => {
    if (isFirstRender.current) {
      isFirstRender.current = false;
      return;
    }
    setAnnouncement(`Showing ${VIEW_LABELS[current]}`);
  }, [current]);

  return (
    <Bar aria-label="Leaderboard view">
      <Title>{current === "groups" ? "Groups" : "Leaderboard"}</Title>
      <Group>
        <Item
          href={buildLeaderboardViewHref(searchParams, "users")}
          $active={current === "users"}
          aria-current={current === "users" ? "page" : undefined}
        >
          Users
        </Item>
        <Item
          href={buildLeaderboardViewHref(searchParams, "groups")}
          $active={current === "groups"}
          aria-current={current === "groups" ? "page" : undefined}
        >
          Groups
        </Item>
      </Group>
      <VisuallyHidden role="status" aria-live="polite">
        {announcement}
      </VisuallyHidden>
    </Bar>
  );
}
